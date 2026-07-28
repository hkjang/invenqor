use crate::model::{AssetChange, AssetRecord, ChangeKind, Envelope, Snapshot};
use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct StateStore {
    root: PathBuf,
    queue: PathBuf,
    max_queue_bytes: u64,
    sequence: Arc<Mutex<u128>>,
}

impl StateStore {
    pub fn open(root: &Path, max_queue_bytes: u64) -> Result<Self> {
        let queue = root.join("queue");
        create_secure_dir(root)?;
        create_secure_dir(&queue)?;
        let existing_sequence = fs::read_dir(&queue)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .split('-')
                    .next()
                    .and_then(|value| value.parse::<u128>().ok())
            })
            .max()
            .unwrap_or(0);
        Ok(Self {
            root: root.to_path_buf(),
            queue,
            max_queue_bytes,
            sequence: Arc::new(Mutex::new(existing_sequence.max(unix_nanos()))),
        })
    }

    pub fn snapshot_hash(snapshot: &Snapshot) -> Result<String> {
        #[derive(Serialize)]
        struct StableRecord<'a> {
            asset_id: &'a str,
            category: &'a str,
            source: &'a str,
            payload: &'a serde_json::Value,
        }

        let records: Vec<_> = snapshot
            .records
            .iter()
            .map(|record| StableRecord {
                asset_id: &record.asset_id,
                category: &record.category,
                source: &record.source,
                payload: &record.payload,
            })
            .collect();
        let stable = StableSnapshotView {
            schema_version: snapshot.schema_version,
            records: &records,
        };
        let bytes = serde_json::to_vec(&stable)?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    pub fn previous_inventory(&self) -> Result<Vec<AssetRecord>> {
        let path = self.root.join("inventory.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
    }

    pub fn set_previous_inventory(&self, records: &[AssetRecord]) -> Result<()> {
        let bytes = serde_json::to_vec(records)?;
        atomic_write(&self.root.join("inventory.json"), &bytes, 0o600)
    }

    pub fn effective_inventory(
        previous: &[AssetRecord],
        current: &[AssetRecord],
        allow_removals: bool,
    ) -> Vec<AssetRecord> {
        if allow_removals {
            return current.to_vec();
        }
        let mut records: std::collections::BTreeMap<_, _> = previous
            .iter()
            .map(|record| (record.asset_id.clone(), record.clone()))
            .collect();
        for record in current {
            records.insert(record.asset_id.clone(), record.clone());
        }
        records.into_values().collect()
    }

    pub fn diff(
        previous: &[AssetRecord],
        current: &[AssetRecord],
        allow_removals: bool,
    ) -> Vec<AssetChange> {
        use std::collections::BTreeMap;
        let previous: BTreeMap<_, _> = previous
            .iter()
            .map(|record| (record.asset_id.as_str(), record))
            .collect();
        let current: BTreeMap<_, _> = current
            .iter()
            .map(|record| (record.asset_id.as_str(), record))
            .collect();
        let mut changes = Vec::new();
        for (asset_id, record) in &current {
            match previous.get(asset_id) {
                None => changes.push(AssetChange {
                    kind: ChangeKind::Added,
                    asset_id: (*asset_id).to_string(),
                    category: record.category.clone(),
                    record: Some((*record).clone()),
                }),
                Some(old) if !same_asset(old, record) => changes.push(AssetChange {
                    kind: ChangeKind::Updated,
                    asset_id: (*asset_id).to_string(),
                    category: record.category.clone(),
                    record: Some((*record).clone()),
                }),
                _ => {}
            }
        }
        if allow_removals {
            for (asset_id, record) in previous {
                if !current.contains_key(asset_id) {
                    changes.push(AssetChange {
                        kind: ChangeKind::Removed,
                        asset_id: asset_id.to_string(),
                        category: record.category.clone(),
                        record: None,
                    });
                }
            }
        }
        changes.sort_by(|a, b| a.asset_id.cmp(&b.asset_id));
        changes
    }

    pub fn previous_hash(&self) -> Option<String> {
        read_trimmed(self.root.join("snapshot.sha256"))
    }

    pub fn set_previous_hash(&self, value: &str) -> Result<()> {
        atomic_write(&self.root.join("snapshot.sha256"), value.as_bytes(), 0o600)
    }

    pub fn last_heartbeat(&self) -> u64 {
        read_trimmed(self.root.join("last-heartbeat"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    pub fn set_last_heartbeat(&self, value: u64) -> Result<()> {
        atomic_write(
            &self.root.join("last-heartbeat"),
            value.to_string().as_bytes(),
            0o600,
        )
    }

    pub fn enqueue(&self, envelope: &Envelope) -> Result<PathBuf> {
        let mut bytes = serde_json::to_vec(envelope)?;
        bytes.push(b'\n');
        let used = self.queue_bytes()?;
        anyhow::ensure!(
            used.saturating_add(bytes.len() as u64) <= self.max_queue_bytes,
            "durable queue is full ({} byte limit); preserving existing events",
            self.max_queue_bytes
        );
        let mut sequence = self
            .sequence
            .lock()
            .map_err(|_| anyhow::anyhow!("queue sequence lock is poisoned"))?;
        *sequence = sequence.saturating_add(1).max(unix_nanos());
        let path = self
            .queue
            .join(format!("{:039}-{}.jsonl", *sequence, envelope.event_id));
        atomic_write(&path, &bytes, 0o600)?;
        Ok(path)
    }

    pub fn pending(&self) -> Result<Vec<PathBuf>> {
        let mut paths: Vec<_> = fs::read_dir(&self.queue)
            .with_context(|| format!("read queue {}", self.queue.display()))?
            .filter_map(|entry| entry.ok().map(|v| v.path()))
            .filter(|path| path.extension().and_then(|v| v.to_str()) == Some("jsonl"))
            .collect();
        paths.sort();
        Ok(paths)
    }

    pub fn read_envelope(&self, path: &Path) -> Result<Envelope> {
        let bytes =
            fs::read(path).with_context(|| format!("read queued event {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("parse queued event {}", path.display()))
    }

    pub fn acknowledge(&self, path: &Path) -> Result<()> {
        anyhow::ensure!(
            path.parent() == Some(self.queue.as_path()),
            "refusing to acknowledge a file outside the queue"
        );
        fs::remove_file(path)
            .with_context(|| format!("remove acknowledged event {}", path.display()))
    }

    pub fn queue_bytes(&self) -> Result<u64> {
        Ok(self
            .pending()?
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|meta| meta.len())
            .sum())
    }
}

// Separate view lets the stable hash omit per-collection timestamps.
#[derive(Serialize)]
struct StableSnapshotView<'a, T: Serialize> {
    schema_version: u32,
    records: &'a [T],
}

fn same_asset(left: &AssetRecord, right: &AssetRecord) -> bool {
    left.asset_id == right.asset_id
        && left.category == right.category
        && left.source == right.source
        && left.payload == right.payload
}

fn create_secure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("secure {}", path.display()))
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".tmp-{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetRecord, EnvelopeKind};
    use serde_json::json;
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("invenqor-test-{}", Uuid::new_v4()))
    }

    #[test]
    fn hash_ignores_collection_time() {
        let record = |time| AssetRecord {
            asset_id: "os".into(),
            category: "os".into(),
            source: "/proc".into(),
            collected_at: time,
            payload: json!({"name": "test"}),
        };
        let snapshot = |time| Snapshot {
            schema_version: 1,
            agent_id: "a".into(),
            collected_at: time,
            duration_ms: time,
            records: vec![record(time)],
            errors: Vec::new(),
        };
        assert_eq!(
            StateStore::snapshot_hash(&snapshot(1)).unwrap(),
            StateStore::snapshot_hash(&snapshot(2)).unwrap()
        );
    }

    #[test]
    fn queue_round_trip_and_acknowledge() {
        let root = temp_dir();
        let store = StateStore::open(&root, 1024 * 1024).unwrap();
        let envelope = Envelope {
            schema_version: 1,
            event_id: Uuid::new_v4().to_string(),
            agent_id: "agent".into(),
            created_at: 1,
            kind: EnvelopeKind::Heartbeat,
            snapshot_hash: "hash".into(),
            snapshot: None,
            changes: Vec::new(),
            collection_errors: Vec::new(),
        };
        let path = store.enqueue(&envelope).unwrap();
        assert_eq!(
            store.read_envelope(&path).unwrap().event_id,
            envelope.event_id
        );
        store.acknowledge(&path).unwrap();
        assert!(store.pending().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queue_preserves_enqueue_order_with_same_event_second() {
        let root = temp_dir();
        let store = StateStore::open(&root, 1024 * 1024).unwrap();
        let envelope = |event_id: &str| Envelope {
            schema_version: 1,
            event_id: event_id.into(),
            agent_id: "agent".into(),
            created_at: 1,
            kind: EnvelopeKind::Heartbeat,
            snapshot_hash: "hash".into(),
            snapshot: None,
            changes: Vec::new(),
            collection_errors: Vec::new(),
        };
        store.enqueue(&envelope("z-first")).unwrap();
        store.enqueue(&envelope("a-second")).unwrap();
        let pending = store.pending().unwrap();
        assert_eq!(
            store.read_envelope(&pending[0]).unwrap().event_id,
            "z-first"
        );
        assert_eq!(
            store.read_envelope(&pending[1]).unwrap().event_id,
            "a-second"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diff_reports_add_update_and_remove() {
        let record = |id: &str, value: u64| AssetRecord {
            asset_id: id.into(),
            category: "test".into(),
            source: "fixture".into(),
            collected_at: value,
            payload: json!({"value": value}),
        };
        let changes = StateStore::diff(
            &[record("removed", 1), record("updated", 1)],
            &[record("added", 1), record("updated", 2)],
            true,
        );
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].asset_id, "added");
        assert_eq!(changes[0].kind, ChangeKind::Added);
        assert_eq!(changes[1].asset_id, "removed");
        assert_eq!(changes[1].kind, ChangeKind::Removed);
        assert_eq!(changes[2].asset_id, "updated");
        assert_eq!(changes[2].kind, ChangeKind::Updated);
    }

    #[test]
    fn partial_collection_does_not_remove_assets() {
        let previous = AssetRecord {
            asset_id: "old".into(),
            category: "test".into(),
            source: "fixture".into(),
            collected_at: 1,
            payload: json!({}),
        };
        assert!(StateStore::diff(std::slice::from_ref(&previous), &[], false).is_empty());
        assert_eq!(
            StateStore::effective_inventory(&[previous], &[], false).len(),
            1
        );
    }
}

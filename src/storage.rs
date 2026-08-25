use crate::health::StatusReport;
use crate::model::{AssetChange, AssetRecord, ChangeKind, Envelope, Snapshot};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct StateStore {
    root: PathBuf,
    queue: PathBuf,
    max_queue_bytes: u64,
    sequence: Arc<Mutex<u128>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ServerCredential {
    server_url: String,
    secret: String,
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

    /// The last inventory this Agent sent, used only to work out what changed.
    ///
    /// An unreadable file is treated as no previous inventory rather than as an
    /// error. It is a cache: losing it costs one full snapshot instead of a
    /// delta, which the Server accepts. Failing instead ended the whole run, and
    /// since the file is read again on the next start, every start after that
    /// ended the same way - a service manager restarting an Agent that could
    /// never do anything, over one damaged cache file.
    pub fn previous_inventory(&self) -> Result<Vec<AssetRecord>> {
        let path = self.root.join("inventory.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "the previous inventory could not be read; sending a full snapshot"
                );
                return Ok(Vec::new());
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(records) => Ok(records),
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "the previous inventory could not be parsed; sending a full snapshot"
                );
                Ok(Vec::new())
            }
        }
    }

    pub fn set_previous_inventory(&self, records: &[AssetRecord]) -> Result<()> {
        let bytes = serde_json::to_vec(records)?;
        atomic_write(&self.root.join("inventory.json"), &bytes)
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
        atomic_write(&self.root.join("snapshot.sha256"), value.as_bytes())
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
        atomic_write(&path, &bytes)?;
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

    /// Moves an event that cannot be read out of the delivery queue.
    ///
    /// The queue is drained oldest first, and an event that fails to parse used
    /// to abort the whole cycle with the file still in place - so the next cycle
    /// reached the same file and failed the same way, and the Agent never
    /// delivered anything again while its queue filled behind the blockage.
    ///
    /// Enqueueing writes atomically, so a half-written file is unlikely; an
    /// event queued by one version and read back by another after an automatic
    /// update is the realistic way to get here, along with anything that damages
    /// the file underneath us.
    ///
    /// Moved rather than deleted: it is collected inventory, and an operator
    /// looking into why an event never arrived needs to see it. It leaves the
    /// queue directory, so it no longer blocks delivery and no longer counts
    /// against the queue limit.
    pub fn quarantine(&self, path: &Path) -> Result<PathBuf> {
        anyhow::ensure!(
            path.parent() == Some(self.queue.as_path()),
            "refusing to quarantine a file outside the queue"
        );
        let directory = self.root.join("queue-unreadable");
        create_secure_dir(&directory)?;
        let name = path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("queued event has no file name"))?;
        let destination = directory.join(name);
        fs::rename(path, &destination).with_context(|| {
            format!("quarantine {} to {}", path.display(), destination.display())
        })?;
        Ok(destination)
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

    pub fn device_token(&self, server_url: &str) -> Option<String> {
        self.read_server_credential("device-credential.json", server_url)
            .map(|credential| credential.secret)
    }

    pub fn set_device_token(&self, server_url: &str, token: &str) -> Result<()> {
        anyhow::ensure!(
            token.starts_with("ivq_at_"),
            "server returned an invalid device token"
        );
        self.write_server_credential("device-credential.json", server_url, token)
    }

    pub fn clear_device_token(&self, server_url: &str) -> Result<()> {
        if self.device_token(server_url).is_none() {
            return Ok(());
        }
        let path = self.root.join("device-credential.json");
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
        }
    }

    pub fn enrollment_claim(&self, server_url: &str) -> Result<String> {
        if let Some(credential) = self.read_server_credential("enrollment-claim.json", server_url) {
            anyhow::ensure!(
                credential.secret.starts_with("ivq_ec_"),
                "stored enrollment claim is invalid"
            );
            return Ok(credential.secret);
        }
        let claim = format!(
            "ivq_ec_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        self.write_server_credential("enrollment-claim.json", server_url, &claim)?;
        Ok(claim)
    }

    pub fn status_path(&self) -> PathBuf {
        self.root.join("status.json")
    }

    /// Persists the operational summary next to the queue it describes. A
    /// failure to write it must never stop collection, so the caller logs and
    /// continues; the report is a diagnosis aid, not durable state.
    pub fn write_status(&self, report: &StatusReport) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(report)?;
        bytes.push(b'\n');
        atomic_write(&self.status_path(), &bytes)
    }

    pub fn read_status(&self) -> Option<StatusReport> {
        serde_json::from_slice(&fs::read(self.status_path()).ok()?).ok()
    }

    fn read_server_credential(&self, name: &str, server_url: &str) -> Option<ServerCredential> {
        let bytes = fs::read(self.root.join(name)).ok()?;
        let credential: ServerCredential = serde_json::from_slice(&bytes).ok()?;
        (credential.server_url == normalized_server_url(server_url)).then_some(credential)
    }

    fn write_server_credential(&self, name: &str, server_url: &str, secret: &str) -> Result<()> {
        let credential = ServerCredential {
            server_url: normalized_server_url(server_url),
            secret: secret.to_string(),
        };
        let bytes = serde_json::to_vec(&credential)?;
        atomic_write(&self.root.join(name), &bytes)
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
    crate::platform::create_private_dir(path)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".tmp-{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let mut file = crate::platform::create_private_file(&temporary)?;
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

fn normalized_server_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
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
    fn persists_credentials_per_server_without_exposing_them_in_config() {
        let root = temp_dir();
        let store = StateStore::open(&root, 1024 * 1024).unwrap();
        let claim = store
            .enrollment_claim("https://inventory.example:7070/")
            .unwrap();
        assert!(claim.starts_with("ivq_ec_"));
        assert_eq!(
            store
                .enrollment_claim("https://inventory.example:7070")
                .unwrap(),
            claim
        );
        store
            .set_device_token("https://inventory.example:7070/", "ivq_at_device-token")
            .unwrap();
        assert_eq!(
            store
                .device_token("https://inventory.example:7070")
                .as_deref(),
            Some("ivq_at_device-token")
        );
        assert!(store.device_token("https://other.example:7070").is_none());
        // A stored device credential must not be world-readable. Windows has no
        // mode; there the installer's ACL on the state directory is what keeps it
        // private, and the directory is checked by --diagnose instead.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(root.join("device-credential.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "the device credential must stay private");
        }
        let _ = fs::remove_dir_all(root);
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

    /// The queue is drained oldest first, and an event that cannot be parsed
    /// used to abort the cycle with the file still in place - so every later
    /// cycle reached the same file and failed identically, and the Agent stopped
    /// delivering anything at all while its queue filled up behind the blockage.
    ///
    /// Writing is atomic, so a torn file is unlikely; reading back an event
    /// queued by a different version after an automatic update is the realistic
    /// way to get one, along with anything that damages the file underneath us.
    #[test]
    fn an_unreadable_event_leaves_the_queue_instead_of_blocking_it() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open(root.path(), 1024 * 1024).unwrap();

        let bad = store.queue.join(format!("{:039}-corrupt.jsonl", 1));
        fs::write(&bad, b"{ this was readable when it was written").unwrap();
        let good = store.queue.join(format!("{:039}-intact.jsonl", 2));
        let envelope = Envelope {
            schema_version: 1,
            event_id: "event-1".into(),
            agent_id: "agent-1".into(),
            created_at: 1,
            kind: EnvelopeKind::Heartbeat,
            snapshot_hash: "hash".into(),
            snapshot: None,
            changes: Vec::new(),
            collection_errors: Vec::new(),
        };
        fs::write(&good, serde_json::to_vec(&envelope).unwrap()).unwrap();

        // Oldest first, so the damaged one is reached before the intact one.
        assert_eq!(store.pending().unwrap(), vec![bad.clone(), good.clone()]);
        assert!(store.read_envelope(&bad).is_err());

        let moved = store.quarantine(&bad).unwrap();
        assert!(moved.exists(), "the event must be kept for inspection");
        assert!(
            !moved.starts_with(&store.queue),
            "a quarantined event must leave the queue directory: {}",
            moved.display()
        );

        assert_eq!(
            store.pending().unwrap(),
            vec![good.clone()],
            "delivery must continue with the events that can still be read"
        );
        assert_eq!(
            store.queue_bytes().unwrap(),
            fs::metadata(&good).unwrap().len(),
            "a quarantined event must stop counting against the queue limit"
        );
        store.read_envelope(&good).unwrap();
    }
}

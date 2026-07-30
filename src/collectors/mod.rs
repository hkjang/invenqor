#[cfg(not(windows))]
mod accounts;
#[cfg(not(windows))]
mod command;
#[cfg(not(windows))]
mod containers;
#[cfg(not(windows))]
mod cpu;
#[cfg(not(windows))]
mod disk;
#[cfg(not(windows))]
mod memory;
#[cfg(not(windows))]
mod network;
#[cfg(not(windows))]
mod os;
#[cfg(not(windows))]
mod packages;
#[cfg(not(windows))]
mod process;
#[cfg(not(windows))]
mod services;
#[cfg(windows)]
mod win;

use crate::config::CollectorConfig;
use crate::model::{unix_time, AssetRecord, CollectionError, Snapshot};
use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;

pub trait Collector: Send + Sync {
    fn name(&self) -> &'static str;
    /// Whether this host can answer this collector at all. A collector that
    /// cannot run reports itself as an error rather than as an empty result,
    /// because an empty result would read as "nothing installed".
    fn is_supported(&self) -> bool {
        cfg!(target_os = "linux") || cfg!(windows)
    }
    fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>>;
}

#[cfg(windows)]
pub fn configured(config: &CollectorConfig) -> Vec<Arc<dyn Collector>> {
    win::configured(config)
}

#[cfg(not(windows))]
pub fn configured(config: &CollectorConfig) -> Vec<Arc<dyn Collector>> {
    let mut result: Vec<Arc<dyn Collector>> = Vec::new();
    macro_rules! enabled {
        ($field:ident, $collector:expr) => {
            if config.$field {
                result.push(Arc::new($collector));
            }
        };
    }
    enabled!(os, os::OsCollector);
    enabled!(cpu, cpu::CpuCollector);
    enabled!(memory, memory::MemoryCollector);
    enabled!(disk, disk::DiskCollector);
    enabled!(network, network::NetworkCollector);
    enabled!(
        process,
        process::ProcessCollector {
            include_cmdline: config.include_process_cmdline,
            max_processes: config.max_processes,
        }
    );
    enabled!(packages, packages::PackageCollector);
    enabled!(services, services::ServiceCollector);
    enabled!(accounts, accounts::AccountCollector);
    enabled!(containers, containers::ContainerCollector);
    result
}

/// How long one collector may take before the cycle gives up on it.
///
/// The design promises that one failing collector does not stop the others, and
/// that held for a collector that *returned* an error. It did not hold for one
/// that never returned: a single blocking system call that waits forever - an
/// unreachable SMB share answering GetDiskFreeSpaceEx, a domain controller
/// resolving group members - stopped the whole cycle, so nothing was ever
/// collected, queued, delivered or registered, on a host whose service looked
/// perfectly healthy. A deadline turns that into one reported error.
const COLLECTOR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Collectors that exceeded the deadline. A blocking task cannot be cancelled, so
/// its thread stays parked; calling it again every cycle would park another one
/// until the pool was exhausted. A collector that hangs is broken, so it is
/// reported and then left alone.
static QUARANTINED: std::sync::Mutex<Option<std::collections::BTreeSet<&'static str>>> =
    std::sync::Mutex::new(None);

fn quarantined(name: &'static str) -> bool {
    QUARANTINED
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|set| set.contains(name)))
        .unwrap_or(false)
}

fn quarantine(name: &'static str) {
    if let Ok(mut guard) = QUARANTINED.lock() {
        guard.get_or_insert_with(Default::default).insert(name);
    }
}

pub async fn collect_all(agent_id: &str, collectors: Vec<Arc<dyn Collector>>) -> Snapshot {
    collect_all_within(agent_id, collectors, COLLECTOR_TIMEOUT).await
}

/// The deadline is a parameter so the behaviour can be tested in milliseconds
/// rather than by waiting out a real minute.
pub async fn collect_all_within(
    agent_id: &str,
    collectors: Vec<Arc<dyn Collector>>,
    timeout: std::time::Duration,
) -> Snapshot {
    let started = Instant::now();
    let collected_at = unix_time();
    let mut tasks = Vec::new();
    let mut errors = Vec::new();

    for collector in collectors {
        let name = collector.name();
        if !collector.is_supported() {
            errors.push(CollectionError {
                collector: name.to_string(),
                message: "unsupported on this host".to_string(),
            });
            continue;
        }
        if quarantined(name) {
            errors.push(CollectionError {
                collector: name.to_string(),
                message: format!(
                    "skipped: it exceeded its {timeout:?} deadline earlier in this process \
                     and is not called again until the Agent restarts"
                ),
            });
            continue;
        }
        tasks.push((
            name,
            tokio::task::spawn_blocking(move || collector.collect(collected_at)),
        ));
    }

    let mut records = Vec::new();
    for (name, task) in tasks {
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(Ok(mut values))) => records.append(&mut values),
            Ok(Ok(Err(error))) => errors.push(CollectionError {
                collector: name.to_string(),
                message: format!("{error:#}"),
            }),
            Ok(Err(error)) => errors.push(CollectionError {
                collector: name.to_string(),
                message: format!("collector task failed: {error}"),
            }),
            Err(_) => {
                quarantine(name);
                errors.push(CollectionError {
                    collector: name.to_string(),
                    message: format!(
                        "did not finish within {timeout:?} and was abandoned; the rest of \
                         this cycle continued without it"
                    ),
                });
            }
        }
    }

    records.sort_by(|a, b| a.asset_id.cmp(&b.asset_id));
    errors.sort_by(|a, b| a.collector.cmp(&b.collector));

    Snapshot {
        schema_version: 1,
        agent_id: agent_id.to_string(),
        collected_at,
        duration_ms: started.elapsed().as_millis() as u64,
        records,
        errors,
    }
}

fn record(
    category: &str,
    source: &str,
    collected_at: u64,
    payload: serde_json::Value,
) -> AssetRecord {
    AssetRecord {
        asset_id: asset_id(category, &payload),
        category: category.to_string(),
        source: source.to_string(),
        collected_at,
        payload,
    }
}

fn asset_id(category: &str, payload: &serde_json::Value) -> String {
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
    };
    let number = |key: &str| {
        payload
            .get(key)
            .and_then(|value| value.as_u64())
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    match category {
        "process" => format!("process:{}:{}", number("pid"), number("start_ticks")),
        "software.package" => format!(
            "package:{}:{}:{}",
            text("manager"),
            text("name"),
            text("architecture")
        ),
        "service" => format!("service:{}:{}", text("manager"), text("name")),
        "hardware.filesystem" => format!("filesystem:{}", text("mountpoint")),
        "network.interface" => format!("interface:{}", text("name")),
        "account.user" => format!("user:{}:{}", number("uid"), text("name")),
        _ => category.to_string(),
    }
}

#[cfg(not(windows))]
fn read_trimmed(path: impl AsRef<std::path::Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Hangs;
    impl Collector for Hangs {
        fn name(&self) -> &'static str {
            "hangs"
        }
        fn collect(&self, _collected_at: u64) -> Result<Vec<AssetRecord>> {
            // Stands in for a blocking call that does not return: an unreachable
            // SMB share answering GetDiskFreeSpaceEx, or a domain controller
            // resolving group members. Well past the deadline the test sets, and
            // short enough that the parked thread does not slow the suite.
            std::thread::sleep(std::time::Duration::from_millis(1_500));
            Ok(Vec::new())
        }
    }

    struct Works;
    impl Collector for Works {
        fn name(&self) -> &'static str {
            "works"
        }
        fn collect(&self, collected_at: u64) -> Result<Vec<AssetRecord>> {
            Ok(vec![record(
                "system",
                "test",
                collected_at,
                serde_json::json!({"hostname": "test-host"}),
            )])
        }
    }

    /// A collector that never returns used to stop the cycle forever: nothing was
    /// collected, queued, delivered or registered, and the only visible sign was
    /// a service that looked healthy and did nothing.
    #[tokio::test]
    async fn a_hanging_collector_does_not_stop_the_others() {
        let deadline = std::time::Duration::from_millis(100);
        let collectors: Vec<Arc<dyn Collector>> = vec![Arc::new(Hangs), Arc::new(Works)];
        let snapshot = collect_all_within("agent-1", collectors, deadline).await;

        // The working collector's records still arrive.
        assert_eq!(snapshot.records.len(), 1, "{:#?}", snapshot);
        // And the hang is reported rather than silently swallowed.
        let reported = snapshot
            .errors
            .iter()
            .find(|error| error.collector == "hangs")
            .expect("the hang must be reported");
        assert!(
            reported.message.contains("did not finish within"),
            "{}",
            reported.message
        );

        // A hung collector must not be called again: each attempt parks another
        // blocking thread, and enough of those exhaust the pool.
        let snapshot =
            collect_all_within("agent-1", vec![Arc::new(Hangs), Arc::new(Works)], deadline).await;
        assert_eq!(snapshot.records.len(), 1);
        let reported = snapshot
            .errors
            .iter()
            .find(|error| error.collector == "hangs")
            .expect("the skip must be reported");
        assert!(reported.message.contains("skipped"), "{}", reported.message);
    }
}

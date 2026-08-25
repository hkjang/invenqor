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

#[cfg(windows)]
pub fn loaded_windows_user_sids() -> std::collections::BTreeSet<String> {
    win::loaded_user_sids()
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
        // Named before it runs, so that when a collector takes the process down -
        // a panic aborts, and a bad system call raises an access violation, and
        // neither leaves anything on a service's discarded standard error - the
        // last line in the log is the collector that did it.
        tracing::info!(collector = name, "collector started");
        tasks.push((
            name,
            tokio::task::spawn_blocking(move || {
                let started = Instant::now();
                let outcome = collector.collect(collected_at);
                let elapsed = started.elapsed().as_millis();
                match &outcome {
                    Ok(records) => tracing::info!(
                        collector = name,
                        records = records.len(),
                        elapsed_ms = elapsed as u64,
                        "collector finished"
                    ),
                    Err(error) => tracing::warn!(
                        collector = name,
                        elapsed_ms = elapsed as u64,
                        error = %format!("{error:#}"),
                        "collector failed"
                    ),
                }
                outcome
            }),
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
        "software.package" => {
            let base = format!(
                "package:{}:{}:{}",
                text("manager"),
                text("name"),
                text("architecture")
            );
            match text("manager") {
                // RPM permits several versions of install-only packages (kernels
                // and gpg-pubkey are common examples) at the same time. Version
                // is therefore part of the package-instance identity there.
                "rpm" => format!("{base}:{}", text("version")),
                // The same Windows product can be installed machine-wide and in
                // several user hives. The registry key is stable across a version
                // update; scope and SID distinguish those real installations.
                "windows" => format!(
                    "package:windows:{}:{}:{}:{}",
                    text("architecture").to_ascii_lowercase(),
                    text("scope").to_ascii_lowercase(),
                    text("owner_sid").to_ascii_lowercase(),
                    text("registry_key").to_ascii_lowercase()
                ),
                _ => base,
            }
        }
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

    #[test]
    fn package_asset_ids_distinguish_coinstalled_rpm_and_windows_instances() {
        let rpm = |version: &str| {
            asset_id(
                "software.package",
                &serde_json::json!({
                    "manager": "rpm",
                    "name": "kernel",
                    "architecture": "x86_64",
                    "version": version,
                }),
            )
        };
        assert_ne!(rpm("0:5.14.0-1"), rpm("0:5.14.0-2"));

        let windows = |name: &str, scope: &str, owner_sid: &str| {
            asset_id(
                "software.package",
                &serde_json::json!({
                    "manager": "windows",
                    "name": name,
                    "architecture": "x64",
                    "scope": scope,
                    "owner_sid": owner_sid,
                    "registry_key": "{00000000-0000-0000-0000-000000000001}",
                }),
            )
        };
        assert_ne!(
            windows("Example Product", "machine", ""),
            windows("Example Product", "user", "S-1-5-21-1")
        );
        assert_ne!(
            windows("Example Product", "user", "S-1-5-21-1"),
            windows("Example Product", "user", "S-1-5-21-2")
        );
        assert_eq!(
            windows("Old Display Name", "user", "S-1-5-21-1"),
            windows("Renamed Product", "user", "S-1-5-21-1"),
            "a mutable display name must not change a registry installation identity"
        );
        assert_eq!(
            windows("Example", "USER", "s-1-5-21-1"),
            windows("Example", "user", "S-1-5-21-1"),
            "case-insensitive registry identity components must be normalized"
        );
    }
}

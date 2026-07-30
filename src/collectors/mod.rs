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

pub async fn collect_all(agent_id: &str, collectors: Vec<Arc<dyn Collector>>) -> Snapshot {
    let started = Instant::now();
    let collected_at = unix_time();
    let mut tasks = Vec::new();
    let mut errors = Vec::new();

    for collector in collectors {
        if !collector.is_supported() {
            errors.push(CollectionError {
                collector: collector.name().to_string(),
                message: "unsupported on this host".to_string(),
            });
            continue;
        }
        let name = collector.name();
        tasks.push((
            name,
            tokio::task::spawn_blocking(move || collector.collect(collected_at)),
        ));
    }

    let mut records = Vec::new();
    for (name, task) in tasks {
        match task.await {
            Ok(Ok(mut values)) => records.append(&mut values),
            Ok(Err(error)) => errors.push(CollectionError {
                collector: name.to_string(),
                message: format!("{error:#}"),
            }),
            Err(error) => errors.push(CollectionError {
                collector: name.to_string(),
                message: format!("collector task failed: {error}"),
            }),
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

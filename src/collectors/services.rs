use super::{record, Collector};
use anyhow::Result;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

pub struct ServiceCollector;

impl Collector for ServiceCollector {
    fn name(&self) -> &'static str {
        "services"
    }

    fn is_supported(&self) -> bool {
        Path::new("/run/systemd/system").exists()
            || Path::new("/run/openrc").exists()
            || Path::new("/etc/init.d").exists()
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        if Path::new("/run/systemd/system").exists() {
            if let Ok(values) = systemd_services(collected_at) {
                return Ok(values);
            }
        }
        if Path::new("/run/openrc").exists() {
            return Ok(openrc_services(collected_at));
        }
        Ok(sysv_services(collected_at))
    }
}

fn systemd_services(collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
    let output = super::command::run(
        "systemctl",
        &[
            "show",
            "--all",
            "--type=service",
            "--property=Id,LoadState,ActiveState,SubState,UnitFileState",
        ],
        Duration::from_secs(15),
    )?;
    anyhow::ensure!(
        output.success,
        "systemctl query failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .split("\n\n")
        .filter_map(|block| {
            let mut id = None;
            let mut load = None;
            let mut active = None;
            let mut sub = None;
            let mut enabled = None;
            for line in block.lines() {
                let (key, value) = line.split_once('=')?;
                match key {
                    "Id" => id = Some(value),
                    "LoadState" => load = Some(value),
                    "ActiveState" => active = Some(value),
                    "SubState" => sub = Some(value),
                    "UnitFileState" => enabled = Some(value),
                    _ => {}
                }
            }
            Some(record(
                "service",
                "systemd D-Bus via systemctl (fixed arguments)",
                collected_at,
                json!({
                    "manager": "systemd",
                    "name": id?,
                    "load_state": load,
                    "active_state": active,
                    "sub_state": sub,
                    "unit_file_state": enabled,
                }),
            ))
        })
        .collect())
}

fn openrc_services(collected_at: u64) -> Vec<crate::model::AssetRecord> {
    let running = super::command::run("rc-status", &["--all"], Duration::from_secs(10))
        .ok()
        .filter(|v| v.success)
        .map(|v| String::from_utf8_lossy(&v.stdout).into_owned())
        .unwrap_or_default();
    init_scripts()
        .into_iter()
        .map(|name| {
            let state = running
                .lines()
                .find(|line| line.split_whitespace().next() == Some(name.as_str()))
                .map(|line| line.trim().to_string());
            record(
                "service",
                "/etc/init.d,rc-status (fixed arguments)",
                collected_at,
                json!({"manager": "openrc", "name": name, "state": state}),
            )
        })
        .collect()
}

fn sysv_services(collected_at: u64) -> Vec<crate::model::AssetRecord> {
    let enabled = sysv_enabled();
    init_scripts()
        .into_iter()
        .map(|name| {
            record(
                "service",
                "/etc/init.d,/etc/rc*.d",
                collected_at,
                json!({
                    "manager": "sysv",
                    "name": name,
                    "enabled_runlevels": enabled.get(&name).cloned().unwrap_or_default(),
                    "active_state": null,
                }),
            )
        })
        .collect()
}

fn init_scripts() -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir("/etc/init.d")
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.') && name != "README")
        .collect();
    names.sort();
    names
}

fn sysv_enabled() -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut result = std::collections::BTreeMap::<String, Vec<u8>>::new();
    for level in 0..=6u8 {
        let path = format!("/etc/rc{level}.d");
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('S') && name.len() > 3 {
                result.entry(name[3..].to_string()).or_default().push(level);
            }
        }
    }
    result
}

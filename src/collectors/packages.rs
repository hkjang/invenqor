use super::{record, Collector};
use anyhow::{Context, Result};
use serde_json::json;
use std::path::Path;
use std::time::Duration;

pub struct PackageCollector;

impl Collector for PackageCollector {
    fn name(&self) -> &'static str {
        "packages"
    }

    fn is_supported(&self) -> bool {
        Path::new("/var/lib/dpkg/status").exists()
            || Path::new("/lib/apk/db/installed").exists()
            || Path::new("/var/lib/rpm").exists()
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        if Path::new("/var/lib/dpkg/status").exists() {
            return collect_dpkg(collected_at);
        }
        if Path::new("/lib/apk/db/installed").exists() {
            return collect_apk(collected_at);
        }
        if Path::new("/var/lib/rpm").exists() {
            return collect_rpm(collected_at);
        }
        Ok(Vec::new())
    }
}

fn collect_dpkg(collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
    let text = std::fs::read_to_string("/var/lib/dpkg/status")?;
    let mut result = Vec::new();
    for block in text.split("\n\n") {
        let fields = deb822_fields(block);
        if !fields
            .iter()
            .any(|(key, value)| key == "Status" && value == "install ok installed")
        {
            continue;
        }
        let get = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        let Some(name) = get("Package") else {
            continue;
        };
        result.push(record(
            "software.package",
            "/var/lib/dpkg/status",
            collected_at,
            json!({
                "manager": "dpkg",
                "name": name,
                "version": get("Version"),
                "architecture": get("Architecture"),
                "source_package": get("Source"),
            }),
        ));
    }
    Ok(result)
}

fn deb822_fields(block: &str) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    for line in block.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = result.last_mut() {
                value.push('\n');
                value.push_str(line.trim());
            }
        } else if let Some((key, value)) = line.split_once(':') {
            result.push((key.to_string(), value.trim().to_string()));
        }
    }
    result
}

fn collect_apk(collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
    let text = std::fs::read_to_string("/lib/apk/db/installed")?;
    let mut result = Vec::new();
    for block in text.split("\n\n") {
        let mut name = None;
        let mut version = None;
        let mut architecture = None;
        for line in block.lines() {
            match line.as_bytes().first() {
                Some(b'P') if line.starts_with("P:") => name = line.get(2..),
                Some(b'V') if line.starts_with("V:") => version = line.get(2..),
                Some(b'A') if line.starts_with("A:") => architecture = line.get(2..),
                _ => {}
            }
        }
        if let Some(name) = name {
            result.push(record(
                "software.package",
                "/lib/apk/db/installed",
                collected_at,
                json!({
                    "manager": "apk",
                    "name": name,
                    "version": version,
                    "architecture": architecture,
                }),
            ));
        }
    }
    Ok(result)
}

fn collect_rpm(collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
    // RPM database formats vary (Berkeley DB, NDB, SQLite). The rpm client is the
    // compatibility boundary; arguments are fixed and no shell is involved.
    let output = super::command::run(
        "rpm",
        &[
            "-qa",
            "--qf",
            "%{NAME}\\t%{EPOCHNUM}:%{VERSION}-%{RELEASE}\\t%{ARCH}\\n",
        ],
        Duration::from_secs(30),
    )
    .context("execute rpm package query")?;
    anyhow::ensure!(
        output.success,
        "rpm query failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?;
            let version = fields.next()?;
            let architecture = fields.next()?;
            Some(record(
                "software.package",
                "rpm -qa (fixed arguments)",
                collected_at,
                json!({
                    "manager": "rpm",
                    "name": name,
                    "version": version,
                    "architecture": architecture,
                }),
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_deb822() {
        let values = deb822_fields("Package: demo\nDescription: first\n second\n");
        assert_eq!(values[0], ("Package".to_string(), "demo".to_string()));
        assert_eq!(values[1].1, "first\nsecond");
    }
}

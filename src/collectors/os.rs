use super::{read_trimmed, record, Collector};
use anyhow::Result;
use serde_json::{json, Map, Value};
use std::path::Path;

pub struct OsCollector;

impl Collector for OsCollector {
    fn name(&self) -> &'static str {
        "os"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let release = parse_os_release(
            &std::fs::read_to_string("/etc/os-release")
                .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
                .unwrap_or_default(),
        );
        let hostname = read_trimmed("/proc/sys/kernel/hostname")
            .or_else(|| read_trimmed("/etc/hostname"))
            .unwrap_or_else(|| "unknown".to_string());
        let kernel_release =
            read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_else(|| "unknown".to_string());
        let kernel_version = read_trimmed("/proc/sys/kernel/version");
        let boot_time = parse_boot_time(&std::fs::read_to_string("/proc/stat").unwrap_or_default());
        let timezone = timezone();

        Ok(vec![record(
            "system",
            "/etc/os-release,/proc",
            collected_at,
            json!({
                "hostname": hostname,
                "architecture": std::env::consts::ARCH,
                "kernel_release": kernel_release,
                "kernel_version": kernel_version,
                "boot_time": boot_time,
                "timezone": timezone,
                "os_release": release,
            }),
        )])
    }
}

pub(crate) fn parse_os_release(text: &str) -> Value {
    let mut result = Map::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                result.insert(key.to_ascii_lowercase(), Value::String(unquote(value)));
            }
        }
    }
    Value::Object(result)
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        value.to_string()
    }
}

fn parse_boot_time(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|v| v.trim().parse().ok())
}

fn timezone() -> Option<String> {
    read_trimmed("/etc/timezone").or_else(|| {
        std::fs::read_link(Path::new("/etc/localtime"))
            .ok()
            .and_then(|path| {
                let value = path.to_string_lossy();
                value
                    .split("/zoneinfo/")
                    .nth(1)
                    .map(std::string::ToString::to_string)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_and_quotes() {
        let value = parse_os_release(
            "# comment\nNAME=\"Example Linux\"\nID=example\nPRETTY_NAME='Example 1'\ninvalid=x\n",
        );
        assert_eq!(value["name"], "Example Linux");
        assert_eq!(value["id"], "example");
        assert_eq!(value["pretty_name"], "Example 1");
        assert!(value.get("invalid").is_none());
    }
}

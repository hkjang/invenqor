use super::{record, Collector};
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeSet;

pub struct CpuCollector;

impl Collector for CpuCollector {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let text = std::fs::read_to_string("/proc/cpuinfo")?;
        let info = parse_cpuinfo(&text);
        let load = std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|v| parse_loadavg(&v));
        Ok(vec![record(
            "hardware.cpu",
            "/proc/cpuinfo,/proc/loadavg",
            collected_at,
            json!({
                "architecture": std::env::consts::ARCH,
                "logical_cpus": info.logical_cpus,
                "physical_packages": info.physical_packages,
                "models": info.models,
                "load_average": load,
            }),
        )])
    }
}

struct CpuInfo {
    logical_cpus: usize,
    physical_packages: usize,
    models: Vec<String>,
}

fn parse_cpuinfo(text: &str) -> CpuInfo {
    let mut logical = 0;
    let mut models = BTreeSet::new();
    let mut packages = BTreeSet::new();
    let mut blocks = 0;
    for block in text.split("\n\n").filter(|v| !v.trim().is_empty()) {
        blocks += 1;
        let mut has_processor = false;
        for line in block.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            match key.trim() {
                "processor" => has_processor = true,
                "model name" | "Hardware" | "Processor" => {
                    let value = value.trim();
                    if !value.is_empty() {
                        models.insert(value.to_string());
                    }
                }
                "physical id" => {
                    packages.insert(value.trim().to_string());
                }
                _ => {}
            }
        }
        if has_processor {
            logical += 1;
        }
    }
    if logical == 0 {
        logical = blocks;
    }
    CpuInfo {
        logical_cpus: logical,
        physical_packages: packages.len().max(if logical > 0 { 1 } else { 0 }),
        models: models.into_iter().collect(),
    }
}

fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let mut fields = text.split_whitespace();
    Some([
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_blocks() {
        let info = parse_cpuinfo(
            "processor: 0\nphysical id: 0\nmodel name: Test\n\nprocessor: 1\nphysical id: 0\nmodel name: Test\n",
        );
        assert_eq!(info.logical_cpus, 2);
        assert_eq!(info.physical_packages, 1);
        assert_eq!(info.models, vec!["Test"]);
    }
}

use super::{record, Collector};
use anyhow::Result;
use serde_json::{json, Map, Value};

pub struct MemoryCollector;

impl Collector for MemoryCollector {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let text = std::fs::read_to_string("/proc/meminfo")?;
        Ok(vec![record(
            "hardware.memory",
            "/proc/meminfo",
            collected_at,
            parse_meminfo(&text),
        )])
    }
}

pub(crate) fn parse_meminfo(text: &str) -> Value {
    let mut values = Map::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let mut fields = value.split_whitespace();
        let Some(number) = fields.next().and_then(|v| v.parse::<u64>().ok()) else {
            continue;
        };
        let multiplier = match fields.next() {
            Some("kB") => 1024,
            Some("mB") | Some("MB") => 1024 * 1024,
            _ => 1,
        };
        values.insert(
            format!("{}_bytes", camel_to_snake(key)),
            json!(number.saturating_mul(multiplier)),
        );
    }
    Value::Object(values)
}

fn camel_to_snake(value: &str) -> String {
    let mut out = String::new();
    for (index, c) in value.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_bytes() {
        let value = parse_meminfo("MemTotal: 100 kB\nSwapTotal: 2 kB\nHugePages_Total: 0\n");
        assert_eq!(value["mem_total_bytes"], 102400);
        assert_eq!(value["swap_total_bytes"], 2048);
    }
}

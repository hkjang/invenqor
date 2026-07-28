use super::{record, Collector};
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;

pub struct AccountCollector;

impl Collector for AccountCollector {
    fn name(&self) -> &'static str {
        "accounts"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let groups = parse_groups(&std::fs::read_to_string("/etc/group").unwrap_or_default());
        let passwd = std::fs::read_to_string("/etc/passwd")?;
        let mut records = Vec::new();
        for line in passwd.lines().filter(|v| !v.trim_start().starts_with('#')) {
            let fields: Vec<_> = line.split(':').collect();
            if fields.len() < 7 {
                continue;
            }
            let name = fields[0];
            let memberships: Vec<_> = groups
                .iter()
                .filter(|(_, members)| members.iter().any(|member| member == name))
                .map(|(group, _)| group.clone())
                .collect();
            records.push(record(
                "account.user",
                "/etc/passwd,/etc/group",
                collected_at,
                json!({
                    "name": name,
                    "uid": fields[2].parse::<u32>().ok(),
                    "gid": fields[3].parse::<u32>().ok(),
                    "gecos": fields[4],
                    "home": fields[5],
                    "shell": fields[6],
                    "supplementary_groups": memberships,
                }),
            ));
        }
        Ok(records)
    }
}

fn parse_groups(text: &str) -> BTreeMap<String, Vec<String>> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(':').collect();
            if fields.len() < 4 {
                return None;
            }
            Some((
                fields[0].to_string(),
                fields[3]
                    .split(',')
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .collect(),
            ))
        })
        .collect()
}

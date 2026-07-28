use super::{record, Collector};
use anyhow::Result;
use serde_json::json;
use std::path::Path;

pub struct ProcessCollector {
    pub include_cmdline: bool,
    pub max_processes: usize,
}

impl Collector for ProcessCollector {
    fn name(&self) -> &'static str {
        "process"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let mut pids: Vec<u32> = std::fs::read_dir("/proc")?
            .filter_map(|entry| entry.ok()?.file_name().to_string_lossy().parse().ok())
            .collect();
        pids.sort_unstable();
        pids.truncate(self.max_processes);

        let mut records = Vec::new();
        for pid in pids {
            if let Some(payload) = read_process(pid, self.include_cmdline) {
                records.push(record("process", "/proc/[pid]", collected_at, payload));
            }
        }
        Ok(records)
    }
}

fn read_process(pid: u32, include_cmdline: bool) -> Option<serde_json::Value> {
    let base = Path::new("/proc").join(pid.to_string());
    let status = std::fs::read_to_string(base.join("status")).ok()?;
    let stat = std::fs::read_to_string(base.join("stat")).ok()?;
    let parsed = parse_stat(&stat)?;
    let (uid, gid) = parse_ids(&status);
    let exe = std::fs::read_link(base.join("exe"))
        .ok()
        .map(|v| v.to_string_lossy().into_owned());
    let cmdline = include_cmdline.then(|| {
        std::fs::read(base.join("cmdline"))
            .unwrap_or_default()
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .collect::<Vec<_>>()
    });
    Some(json!({
        "pid": pid,
        "name": parsed.name,
        "state": parsed.state,
        "parent_pid": parsed.parent_pid,
        "start_ticks": parsed.start_ticks,
        "uid": uid,
        "gid": gid,
        "executable": exe,
        "cmdline": cmdline,
    }))
}

struct ProcessStat {
    name: String,
    state: String,
    parent_pid: u32,
    start_ticks: u64,
}

fn parse_stat(text: &str) -> Option<ProcessStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let name = text[open + 1..close].to_string();
    let fields: Vec<_> = text[close + 1..].split_whitespace().collect();
    Some(ProcessStat {
        name,
        state: fields.first()?.to_string(),
        parent_pid: fields.get(1)?.parse().ok()?,
        // starttime is field 22; fields begins at field 3.
        start_ticks: fields.get(19)?.parse().ok()?,
    })
}

fn parse_ids(status: &str) -> (Option<u32>, Option<u32>) {
    let first_id = |prefix: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
    };
    (first_id("Uid:"), first_id("Gid:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_names_may_contain_spaces_and_parens() {
        let mut fields = vec!["S", "1"];
        fields.extend(std::iter::repeat("0").take(17));
        fields.push("12345");
        let text = format!("42 (a weird) name) {}", fields.join(" "));
        let stat = parse_stat(&text).unwrap();
        assert_eq!(stat.name, "a weird) name");
        assert_eq!(stat.parent_pid, 1);
        assert_eq!(stat.start_ticks, 12345);
    }
}

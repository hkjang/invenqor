use super::{record, Collector};
use anyhow::Result;
use serde_json::json;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub struct DiskCollector;

impl Collector for DiskCollector {
    fn name(&self) -> &'static str {
        "disk"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let mounts = std::fs::read_to_string("/proc/self/mounts")
            .or_else(|_| std::fs::read_to_string("/proc/mounts"))?;
        let mut records = Vec::new();
        for mount in parse_mounts(&mounts) {
            let usage = statvfs(Path::new(&mount.target));
            records.push(record(
                "hardware.filesystem",
                "/proc/self/mounts",
                collected_at,
                json!({
                    "device": mount.device,
                    "mountpoint": mount.target,
                    "filesystem": mount.filesystem,
                    "options": mount.options,
                    "usage": usage,
                }),
            ));
        }
        Ok(records)
    }
}

struct Mount {
    device: String,
    target: String,
    filesystem: String,
    options: Vec<String>,
}

fn parse_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            Some(Mount {
                device: unescape(f.next()?),
                target: unescape(f.next()?),
                filesystem: f.next()?.to_string(),
                options: f.next()?.split(',').map(str::to_string).collect(),
            })
        })
        .collect()
}

fn unescape(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn statvfs(path: &Path) -> Option<serde_json::Value> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut value = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: path is NUL-terminated and value points to writable memory of the expected type.
    let result = unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    // SAFETY: a successful statvfs call initialized value.
    let value = unsafe { value.assume_init() };
    let block_size = value.f_frsize;
    Some(json!({
        "total_bytes": value.f_blocks.saturating_mul(block_size),
        "available_bytes": value.f_bavail.saturating_mul(block_size),
        "free_bytes": value.f_bfree.saturating_mul(block_size),
        "files": value.f_files,
        "files_free": value.f_ffree,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_escapes_are_decoded() {
        let values = parse_mounts("/dev/sda /a\\040b ext4 rw,relatime 0 0\n");
        assert_eq!(values[0].target, "/a b");
        assert_eq!(values[0].options, vec!["rw", "relatime"]);
    }
}

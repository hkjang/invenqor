use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct HostIdentity {
    pub agent_id: String,
    pub machine_id: Option<String>,
    pub dmi_product_uuid: Option<String>,
}

pub fn load_or_create(state_dir: &Path) -> Result<HostIdentity> {
    fs::create_dir_all(state_dir)
        .with_context(|| format!("create state directory {}", state_dir.display()))?;
    fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("secure state directory {}", state_dir.display()))?;

    let id_path = state_dir.join("agent-id");
    let agent_id = match read_trimmed(&id_path) {
        Some(value) => {
            Uuid::parse_str(&value).context("agent-id is not a valid UUID")?;
            value
        }
        None => create_id(&id_path)?,
    };

    Ok(HostIdentity {
        agent_id,
        machine_id: read_trimmed(Path::new("/etc/machine-id"))
            .or_else(|| read_trimmed(Path::new("/var/lib/dbus/machine-id"))),
        dmi_product_uuid: read_trimmed(Path::new("/sys/class/dmi/id/product_uuid")),
    })
}

fn create_id(path: &Path) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let tmp = temp_path(path);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("create {}", tmp.display()))?;
    writeln!(file, "{id}")?;
    file.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("install {}", path.display()))?;
    Ok(id)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(format!(".tmp-{}", std::process::id()));
    PathBuf::from(value)
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

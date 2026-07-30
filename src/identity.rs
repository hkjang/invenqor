use crate::platform;
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct HostIdentity {
    pub agent_id: String,
    pub machine_id: Option<String>,
    pub dmi_product_uuid: Option<String>,
}

pub fn load_or_create(state_dir: &Path) -> Result<HostIdentity> {
    platform::create_private_dir(state_dir)
        .with_context(|| format!("prepare state directory {}", state_dir.display()))?;

    let id_path = state_dir.join("agent-id");
    let agent_id = match read_trimmed(&id_path) {
        Some(value) => {
            Uuid::parse_str(&value).context("agent-id is not a valid UUID")?;
            value
        }
        None => create_id(&id_path)?,
    };

    let identifiers = platform::machine_identifiers();
    Ok(HostIdentity {
        agent_id,
        machine_id: identifiers.machine_id,
        dmi_product_uuid: identifiers.firmware_uuid,
    })
}

fn create_id(path: &Path) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let tmp = temp_path(path);
    let mut file = platform::create_private_file(&tmp)?;
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

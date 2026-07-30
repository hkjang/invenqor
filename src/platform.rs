//! Everything that differs between the operating systems the Agent runs on.
//!
//! The rest of the Agent is written once. Where Linux and Windows genuinely
//! disagree - where files live, how a file is kept private, what identifies the
//! machine, how a service is restarted - the difference is resolved here rather
//! than scattered through `#[cfg]` blocks in collectors and schedulers.

use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

/// Where an unconfigured install expects its configuration file.
pub fn default_config_path() -> PathBuf {
    #[cfg(windows)]
    {
        program_data().join("Invenqor").join("config.toml")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/invenqor-agent/config.toml")
    }
}

/// Where the identity, inventory hash and durable queue live.
pub fn default_state_dir() -> PathBuf {
    #[cfg(windows)]
    {
        program_data().join("Invenqor").join("state")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/invenqor-agent")
    }
}

/// Where the running executable is expected to be installed. The updater uses
/// it when the configuration does not name a path.
pub fn default_install_path() -> PathBuf {
    #[cfg(windows)]
    {
        program_files().join("Invenqor").join("invenqor-agent.exe")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/opt/invenqor-agent/bin/invenqor-agent")
    }
}

/// The service account the packaged installer runs the Agent as.
pub fn service_account() -> &'static str {
    #[cfg(windows)]
    {
        // The packaged service runs as LocalSystem: an inventory agent needs to
        // read the SCM, every user profile's software registry and adapter
        // configuration, and a lesser account cannot.
        "LocalSystem"
    }
    #[cfg(not(windows))]
    {
        "invenqor-agent"
    }
}

/// The command an operator runs to restart the service on this platform.
pub fn restart_command() -> &'static str {
    #[cfg(windows)]
    {
        "Restart-Service invenqor-agent"
    }
    #[cfg(not(windows))]
    {
        "sudo systemctl restart invenqor-agent"
    }
}

#[cfg(windows)]
fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

#[cfg(windows)]
fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

/// Creates a directory only the service account and administrators can read.
///
/// On Linux that is mode 0700. Windows has no mode; a directory under
/// ProgramData inherits an ACL that grants Users write access to new files,
/// which would leave the queue and the device credential world-readable. The
/// installer sets the restrictive ACL once, and this call keeps directories the
/// Agent creates later inside that inheritance rather than re-deriving an ACL
/// the Agent has no business authoring.
pub fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    restrict_directory(path)
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("secure {}", path.display()))
}

#[cfg(windows)]
fn restrict_directory(path: &Path) -> Result<()> {
    // Inherited from the installer's ACL on %ProgramData%\Invenqor. Nothing to
    // do, and nothing to pretend: see docs for what the installer applies.
    let _ = path;
    Ok(())
}

/// Opens a new file that only the service account and administrators can read.
pub fn create_private_file(path: &Path) -> Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    apply_private_mode(&mut options);
    options
        .open(path)
        .with_context(|| format!("create {}", path.display()))
}

#[cfg(unix)]
fn apply_private_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(windows)]
fn apply_private_mode(_options: &mut OpenOptions) {
    // Windows files inherit the parent directory's ACL, which the installer
    // restricts to SYSTEM and Administrators.
}

/// Marks a file executable. A no-op on Windows, where the extension decides.
pub fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("mark {} executable", path.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// The machine's own name, without shelling out to `hostname`.
pub fn hostname() -> String {
    #[cfg(windows)]
    {
        // COMPUTERNAME is set by the session; the registry value is what the
        // machine is actually called, and a service has no interactive session.
        crate::windows_sys::registry_string(
            crate::windows_sys::HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\ComputerName\ActiveComputerName",
            "ComputerName",
        )
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "unknown-host".to_string())
    }
    #[cfg(not(windows))]
    {
        fs::read_to_string("/proc/sys/kernel/hostname")
            .or_else(|_| fs::read_to_string("/etc/hostname"))
            .map(|value| value.trim().to_string())
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown-host".to_string())
    }
}

/// What identifies this machine beyond the Agent's own generated id, so the
/// Server can recognise a rebuilt or cloned host.
pub struct MachineIdentifiers {
    /// A stable installation identifier: /etc/machine-id, or MachineGuid.
    pub machine_id: Option<String>,
    /// The firmware UUID, which survives a reinstall of the operating system.
    pub firmware_uuid: Option<String>,
}

pub fn machine_identifiers() -> MachineIdentifiers {
    #[cfg(windows)]
    {
        MachineIdentifiers {
            machine_id: crate::windows_sys::registry_string(
                crate::windows_sys::HKEY_LOCAL_MACHINE,
                r"SOFTWARE\Microsoft\Cryptography",
                "MachineGuid",
            ),
            firmware_uuid: crate::windows_sys::smbios_system_uuid(),
        }
    }
    #[cfg(not(windows))]
    {
        MachineIdentifiers {
            machine_id: read_trimmed(Path::new("/etc/machine-id"))
                .or_else(|| read_trimmed(Path::new("/var/lib/dbus/machine-id"))),
            firmware_uuid: read_trimmed(Path::new("/sys/class/dmi/id/product_uuid")),
        }
    }
}

#[cfg(not(windows))]
fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_absolute_and_platform_appropriate() {
        for path in [
            default_config_path(),
            default_state_dir(),
            default_install_path(),
        ] {
            assert!(path.is_absolute(), "{} must be absolute", path.display());
        }
        if cfg!(windows) {
            assert!(default_install_path()
                .extension()
                .is_some_and(|v| v == "exe"));
        } else {
            assert!(default_config_path().starts_with("/etc"));
        }
    }

    #[test]
    fn the_host_reports_a_usable_name() {
        let name = hostname();
        assert!(!name.is_empty());
        assert!(!name.contains('\n'), "hostname must be trimmed: {name:?}");
    }
}

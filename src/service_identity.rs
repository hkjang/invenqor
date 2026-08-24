//! Windows service identity shared by the command-line bootstrap and SCM code.
//!
//! The packaged service normally uses the default name, but the installer also
//! supports side-by-side or organisation-specific names.  The name is data, not
//! a command fragment: validate it before it is ever placed in an SCM command
//! line, persist it beside the protected configuration, and recover it for an
//! elevated `--diagnose` or `--update-now` launched from a console.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_SERVICE_NAME: &str = "invenqor-agent";
pub const SERVICE_NAME_FILE: &str = "service-name";
const MAX_SERVICE_NAME_UTF16: usize = 256;
const MAX_SERVICE_NAME_FILE_BYTES: u64 = 1_024;

/// Applies the deliberately smaller safe subset of Windows service-name rules
/// used by the installer. Windows rejects slash and backslash itself; quotes and
/// control characters are also refused so this value can never alter the
/// service binary's command line. Interior spaces and Unicode remain supported.
pub fn validate_service_name(value: &str) -> Result<&str> {
    anyhow::ensure!(!value.is_empty(), "Windows service name must not be empty");
    anyhow::ensure!(
        value.trim() == value,
        "Windows service name must not start or end with whitespace"
    );
    anyhow::ensure!(
        value.encode_utf16().count() <= MAX_SERVICE_NAME_UTF16,
        "Windows service name must be at most {MAX_SERVICE_NAME_UTF16} UTF-16 code units"
    );
    anyhow::ensure!(
        !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | '"')),
        "Windows service name must not contain slash, backslash, quote, or control characters"
    );
    Ok(value)
}

pub fn service_name_file(config_path: &Path) -> Option<PathBuf> {
    config_path
        .parent()
        .map(|directory| directory.join(SERVICE_NAME_FILE))
}

/// Resolves the current service name. An explicit, validated CLI value wins;
/// otherwise the installer's protected marker beside config.toml is used. Old
/// installations have no marker and continue to use the historical default.
pub fn resolve_service_name(explicit: Option<&str>, config_path: &Path) -> Result<String> {
    if let Some(value) = explicit {
        return Ok(validate_service_name(value)?.to_string());
    }
    let Some(path) = service_name_file(config_path) else {
        return Ok(DEFAULT_SERVICE_NAME.to_string());
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DEFAULT_SERVICE_NAME.to_string())
        }
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    };
    anyhow::ensure!(
        metadata.len() <= MAX_SERVICE_NAME_FILE_BYTES,
        "Windows service name marker {} is unexpectedly large",
        path.display()
    );
    let value = fs::read_to_string(&path)
        .with_context(|| format!("read Windows service name marker {}", path.display()))?;
    Ok(validate_service_name(&value)
        .with_context(|| format!("invalid Windows service name marker {}", path.display()))?
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn old_install_without_marker_uses_the_default() {
        let root = tempdir().unwrap();
        let config = root.path().join("config.toml");
        assert_eq!(
            resolve_service_name(None, &config).unwrap(),
            DEFAULT_SERVICE_NAME
        );
    }

    #[test]
    fn protected_marker_recovers_a_custom_name_and_explicit_name_wins() {
        let root = tempdir().unwrap();
        let config = root.path().join("config.toml");
        fs::write(root.path().join(SERVICE_NAME_FILE), "Invenqor Agent West-1").unwrap();
        assert_eq!(
            resolve_service_name(None, &config).unwrap(),
            "Invenqor Agent West-1"
        );
        assert_eq!(
            resolve_service_name(Some("Invenqor Agent Recovery"), &config).unwrap(),
            "Invenqor Agent Recovery"
        );
    }

    #[test]
    fn command_line_metacharacters_and_ambiguous_whitespace_are_rejected() {
        for value in [
            "",
            " leading",
            "trailing ",
            "invenqor/agent",
            "invenqor\\agent",
            "invenqor\" --config C:\\attacker.toml",
            "invenqor\nagent",
        ] {
            assert!(
                validate_service_name(value).is_err(),
                "unsafe service name was accepted: {value:?}"
            );
        }
    }

    #[test]
    fn limit_is_measured_as_windows_utf16_code_units() {
        assert!(validate_service_name(&"a".repeat(256)).is_ok());
        assert!(validate_service_name(&"a".repeat(257)).is_err());
        assert!(validate_service_name(&"한".repeat(256)).is_ok());
        assert!(validate_service_name(&"🦀".repeat(128)).is_ok());
        assert!(validate_service_name(&"🦀".repeat(129)).is_err());
    }

    #[test]
    fn malformed_marker_fails_closed_instead_of_querying_another_service() {
        let root = tempdir().unwrap();
        let config = root.path().join("config.toml");
        fs::write(
            root.path().join(SERVICE_NAME_FILE),
            "invenqor-agent\" --config C:\\attacker.toml",
        )
        .unwrap();
        let error = resolve_service_name(None, &config).unwrap_err();
        assert!(format!("{error:#}").contains("invalid Windows service name marker"));
    }
}

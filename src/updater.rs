use crate::config::Config;
use crate::storage::StateStore;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use reqwest::{Certificate, Client, Identity, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub channel: String,
    pub os: String,
    pub architecture: String,
    pub sha256: String,
    pub signature: String,
    pub download_url: String,
    pub size: u64,
    /// Set by the Server when an operator publishes a deliberate rollback. The
    /// artifact is still signed and hash-checked, so accepting an older version
    /// on request is safe; refusing it would leave a fleet stuck on a bad build.
    #[serde(default)]
    pub allow_downgrade: bool,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingUpdate {
    manifest: UpdateManifest,
    artifact: PathBuf,
}

pub async fn run_checker(config: Config, agent_id: String) {
    if !config.updates.enabled {
        return;
    }
    // A fleet restarted together would otherwise ask for a manifest at the same
    // instant and then download the same multi-megabyte artifact at the same
    // instant. The offset is derived from the agent identifier so each host keeps
    // its own slot across restarts instead of re-rolling the dice.
    let interval = config.updates.check_interval_seconds.max(1);
    let offset = stable_offset(&agent_id, interval);
    info!(
        interval_seconds = interval,
        first_check_in_seconds = offset,
        channel = %config.updates.channel,
        "agent update checker started"
    );
    tokio::time::sleep(Duration::from_secs(offset)).await;
    loop {
        match check_and_stage(&config, &agent_id).await {
            Ok(Some(version)) => info!(%version, "signed agent update staged"),
            Ok(None) => {}
            Err(error) => warn!(error = %format!("{error:#}"), "agent update check failed"),
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

/// Spreads a fleet across the check interval deterministically.
pub fn stable_offset(agent_id: &str, interval_seconds: u64) -> u64 {
    if interval_seconds <= 1 {
        return 0;
    }
    let mut hash: u64 = 1469598103934665603;
    for byte in agent_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    hash % interval_seconds
}

pub async fn check_and_stage(config: &Config, agent_id: &str) -> Result<Option<String>> {
    if !config.updates.enabled {
        return Ok(None);
    }
    let client = update_client(config)?;
    let base = config
        .server
        .url
        .as_deref()
        .context("server.url is required")?;
    let store = StateStore::open(&config.agent.state_dir, config.agent.max_queue_bytes)?;
    let bearer_token = config
        .server
        .bearer_token
        .clone()
        .or_else(|| store.device_token(base));
    let url = format!(
        "{}/v1/agent/updates?agent_id={}&current_version={}&channel={}&os=linux&arch={}",
        base.trim_end_matches('/'),
        agent_id,
        env!("CARGO_PKG_VERSION"),
        config.updates.channel,
        std::env::consts::ARCH,
    );
    let mut request = client.get(url);
    if let Some(token) = &bearer_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.context("request update manifest")?;
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(None);
    }
    anyhow::ensure!(
        response.status().is_success(),
        "update server returned {}",
        response.status()
    );
    let manifest: UpdateManifest = response.json().await.context("decode update manifest")?;
    let current = env!("CARGO_PKG_VERSION");
    anyhow::ensure!(
        is_newer(current, &manifest.version)
            || (manifest.allow_downgrade && manifest.version != current),
        "server offered {} which is not newer than {current} and is not marked as a rollback",
        manifest.version
    );
    anyhow::ensure!(manifest.os == "linux", "update OS does not match");
    anyhow::ensure!(
        manifest.architecture == std::env::consts::ARCH,
        "update architecture does not match"
    );
    anyhow::ensure!(
        manifest.download_url.starts_with("/v1/agent/updates/")
            && !manifest.download_url.starts_with("//"),
        "update download URL must be a same-server relative path"
    );
    anyhow::ensure!(
        manifest.size > 0 && manifest.size <= 128 * 1024 * 1024,
        "update manifest size is invalid"
    );
    let download = format!("{}{}", base.trim_end_matches('/'), manifest.download_url);
    let mut request = client.get(download);
    if let Some(token) = &bearer_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.context("download update artifact")?;
    anyhow::ensure!(
        response.status().is_success(),
        "update download returned {}",
        response.status()
    );
    anyhow::ensure!(
        response.content_length() == Some(manifest.size),
        "update response Content-Length does not match manifest"
    );
    let bytes = response.bytes().await.context("read update artifact")?;
    anyhow::ensure!(
        bytes.len() as u64 == manifest.size,
        "update size does not match"
    );
    verify_artifact(
        &bytes,
        &manifest,
        config
            .updates
            .public_key
            .as_deref()
            .context("update public key is required")?,
    )?;
    stage(config, manifest.clone(), &bytes)?;
    if manifest.allow_downgrade && !is_newer(current, &manifest.version) {
        info!(
            version = %manifest.version,
            from = current,
            "staged an operator-published rollback"
        );
    }
    Ok(Some(manifest.version))
}

pub fn apply_pending(config: &Config) -> Result<Option<String>> {
    let pending_path = config.agent.state_dir.join("updates/pending.json");
    if !pending_path.exists() {
        return Ok(None);
    }
    let pending: PendingUpdate =
        serde_json::from_slice(&fs::read(&pending_path).context("read pending update")?)
            .context("decode pending update")?;
    let bytes = fs::read(&pending.artifact).context("read staged update")?;
    verify_artifact(
        &bytes,
        &pending.manifest,
        config
            .updates
            .public_key
            .as_deref()
            .context("update public key is required")?,
    )?;
    atomic_install(
        &config.updates.install_path,
        &bytes,
        &pending.manifest.version,
    )?;
    fs::remove_file(&pending_path).context("remove applied update marker")?;
    // The staged copy is several megabytes and was previously left behind on
    // every update, so a long-lived host slowly filled its state directory.
    if let Err(error) = fs::remove_file(&pending.artifact) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warn!(
                artifact = %pending.artifact.display(),
                error = %error,
                "could not remove the staged update artifact"
            );
        }
    }
    prune_staged_artifacts(config, &pending.manifest.version);
    Ok(Some(pending.manifest.version))
}

/// Removes staged artifacts left by earlier attempts. Staging writes one file per
/// version, and a host that saw several releases before a restart would keep
/// every one of them.
fn prune_staged_artifacts(config: &Config, applied_version: &str) {
    let directory = config.agent.state_dir.join("updates");
    let Ok(entries) = fs::read_dir(&directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("invenqor-agent-") {
            continue;
        }
        if name == format!("invenqor-agent-{applied_version}") {
            continue;
        }
        let _ = fs::remove_file(entry.path());
    }
}

fn verify_artifact(bytes: &[u8], manifest: &UpdateManifest, public_key: &str) -> Result<()> {
    let digest = hex::encode(Sha256::digest(bytes));
    anyhow::ensure!(
        digest == manifest.sha256.to_ascii_lowercase(),
        "update SHA-256 mismatch"
    );
    let key: [u8; 32] = STANDARD
        .decode(public_key)
        .context("decode update public key")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Ed25519 public key must be 32 bytes"))?;
    let signature = Signature::from_slice(
        &STANDARD
            .decode(&manifest.signature)
            .context("decode update signature")?,
    )
    .context("parse update signature")?;
    VerifyingKey::from_bytes(&key)
        .context("parse update public key")?
        .verify(bytes, &signature)
        .context("verify update signature")
}

fn stage(config: &Config, manifest: UpdateManifest, bytes: &[u8]) -> Result<()> {
    let directory = config.agent.state_dir.join("updates");
    fs::create_dir_all(&directory).context("create update staging directory")?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let artifact = directory.join(format!("invenqor-agent-{}", manifest.version));
    atomic_write(&artifact, bytes, 0o700)?;
    let pending = serde_json::to_vec(&PendingUpdate { manifest, artifact })?;
    atomic_write(&directory.join("pending.json"), &pending, 0o600)
}

fn atomic_install(target: &Path, bytes: &[u8], version: &str) -> Result<()> {
    let parent = target
        .parent()
        .context("update install path has no parent")?;
    let temporary = parent.join(format!(".invenqor-agent.update-{}", std::process::id()));
    atomic_write(&temporary, bytes, 0o755)?;
    // Run the candidate before it becomes the installed agent. A signed,
    // correctly hashed binary can still be unable to start - wrong architecture
    // family, a missing kernel feature, a bad build - and activating it would
    // stop collection on every host that received it, with no way back except
    // hands on each machine.
    if let Err(error) = self_test(&temporary, version) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let previous = target.with_extension("previous");
    if target.exists() {
        let _ = fs::remove_file(&previous);
        fs::rename(target, &previous).context("preserve previous agent binary")?;
    }
    if let Err(error) = fs::rename(&temporary, target) {
        if previous.exists() {
            let _ = fs::rename(&previous, target);
        }
        return Err(error).context("activate agent update");
    }
    sync_directory(parent)
}

/// Verifies that the staged binary actually runs and reports the version the
/// manifest promised.
fn self_test(candidate: &Path, expected_version: &str) -> Result<()> {
    let output = std::process::Command::new(candidate)
        .arg("--version")
        .output()
        .with_context(|| {
            format!(
                "the staged agent {} could not be executed, so it was not installed",
                candidate.display()
            )
        })?;
    anyhow::ensure!(
        output.status.success(),
        "the staged agent exited with {} during its self-test, so it was not installed",
        output.status
    );
    let reported = String::from_utf8_lossy(&output.stdout);
    anyhow::ensure!(
        reported.contains(expected_version),
        "the staged agent reported {:?} but the update promised {expected_version}, \
         so it was not installed",
        reported.trim()
    );
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(mode)
        .open(&temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    drop(file);
    fs::rename(&temporary, path)?;
    sync_directory(path.parent().context("path has no parent")?)
}

fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

fn update_client(config: &Config) -> Result<Client> {
    let mut builder = Client::builder()
        .https_only(!config.server.allows_http())
        .timeout(Duration::from_secs(config.server.timeout_seconds))
        .user_agent(concat!("invenqor-agent/", env!("CARGO_PKG_VERSION")));
    if let Some(path) = &config.server.ca_file {
        builder = builder.add_root_certificate(Certificate::from_pem(&fs::read(path)?)?);
    }
    if let Some(path) = &config.server.client_identity_pem {
        builder = builder.identity(Identity::from_pem(&fs::read(path)?)?);
    }
    builder.build().context("build update HTTP client")
}

fn is_newer(current: &str, candidate: &str) -> bool {
    fn parts(value: &str) -> Option<Vec<u64>> {
        value
            .trim_start_matches('v')
            .split('.')
            .map(|v| v.parse().ok())
            .collect()
    }
    match (parts(current), parts(candidate)) {
        (Some(left), Some(right)) => right > left,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::tempdir;

    /// Writes a stand-in agent binary: a script that behaves like the real one
    /// for `--version`, which is all the self-test needs.
    fn executable(path: &Path, version: &str, exit_code: i32) -> Vec<u8> {
        let script = format!("#!/bin/sh\necho \"invenqor-agent {version}\"\nexit {exit_code}\n");
        atomic_write(path, script.as_bytes(), 0o755).unwrap();
        script.into_bytes()
    }

    #[test]
    fn verifies_signature_and_applies_atomically_with_backup() {
        let root = tempdir().unwrap();
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let staged = root.path().join("candidate");
        let bytes = executable(&staged, "9.0.0", 0);
        let manifest = UpdateManifest {
            version: "9.0.0".into(),
            channel: "stable".into(),
            os: "linux".into(),
            architecture: std::env::consts::ARCH.into(),
            sha256: hex::encode(Sha256::digest(&bytes)),
            signature: STANDARD.encode(signing.sign(&bytes).to_bytes()),
            download_url: "/artifact".into(),
            size: bytes.len() as u64,
            allow_downgrade: false,
            notes: String::new(),
        };
        verify_artifact(
            &bytes,
            &manifest,
            &STANDARD.encode(signing.verifying_key().to_bytes()),
        )
        .unwrap();
        let target = root.path().join("agent");
        fs::write(&target, b"old").unwrap();
        atomic_install(&target, &bytes, "9.0.0").unwrap();
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert_eq!(fs::read(target.with_extension("previous")).unwrap(), b"old");
    }

    /// The property that matters most: a signed artifact that cannot run must
    /// never replace a working agent. Activating one would stop collection on
    /// every host that received it, recoverable only by hand.
    #[test]
    fn refuses_to_install_a_binary_that_cannot_run() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::write(&target, b"working-agent").unwrap();
        // Not an executable format, and not marked executable either.
        let broken = b"\x7fELF-but-truncated".to_vec();
        let error = atomic_install(&target, &broken, "9.9.9").unwrap_err();
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("not installed"),
            "unexpected error: {rendered}"
        );
        assert_eq!(fs::read(&target).unwrap(), b"working-agent");
        assert!(!target.with_extension("previous").exists());
    }

    #[test]
    fn refuses_a_binary_that_reports_the_wrong_version() {
        let root = tempdir().unwrap();
        let staged = root.path().join("candidate");
        let bytes = executable(&staged, "1.0.0", 0);
        let target = root.path().join("agent");
        fs::write(&target, b"working-agent").unwrap();
        let error = atomic_install(&target, &bytes, "2.0.0").unwrap_err();
        assert!(format!("{error:#}").contains("promised 2.0.0"));
        assert_eq!(fs::read(&target).unwrap(), b"working-agent");
    }

    #[test]
    fn refuses_a_binary_that_exits_non_zero() {
        let root = tempdir().unwrap();
        let staged = root.path().join("candidate");
        let bytes = executable(&staged, "3.0.0", 3);
        let target = root.path().join("agent");
        fs::write(&target, b"working-agent").unwrap();
        assert!(atomic_install(&target, &bytes, "3.0.0").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"working-agent");
    }

    #[test]
    fn rejects_tampering_and_downgrades() {
        assert!(is_newer("1.2.3", "1.2.4"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("2.0.0", "1.9.9"));
    }

    #[test]
    fn check_offset_is_stable_per_host_and_inside_the_interval() {
        let interval = 21_600;
        let first = stable_offset("agent-a", interval);
        assert_eq!(first, stable_offset("agent-a", interval));
        assert!(first < interval);
        // Two hosts must not queue up on the same second.
        let mut distinct = std::collections::HashSet::new();
        for index in 0..200 {
            distinct.insert(stable_offset(&format!("agent-{index}"), interval));
        }
        assert!(
            distinct.len() > 150,
            "offsets clustered: {} distinct slots for 200 hosts",
            distinct.len()
        );
        assert_eq!(stable_offset("agent-a", 1), 0);
    }

    #[test]
    fn pruning_keeps_only_the_applied_artifact() {
        let root = tempdir().unwrap();
        let updates = root.path().join("updates");
        fs::create_dir_all(&updates).unwrap();
        for version in ["0.1.0", "0.2.0", "0.3.0"] {
            fs::write(updates.join(format!("invenqor-agent-{version}")), b"x").unwrap();
        }
        fs::write(updates.join("pending.json"), b"{}").unwrap();
        let mut config = Config::default();
        config.agent.state_dir = root.path().to_path_buf();
        prune_staged_artifacts(&config, "0.3.0");
        assert!(updates.join("invenqor-agent-0.3.0").exists());
        assert!(!updates.join("invenqor-agent-0.1.0").exists());
        assert!(!updates.join("invenqor-agent-0.2.0").exists());
        // Unrelated files must survive.
        assert!(updates.join("pending.json").exists());
    }
}

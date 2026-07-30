use crate::config::Config;
use crate::storage::StateStore;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use reqwest::{Certificate, Client, Identity, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
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

/// The platform name the Server publishes releases under.
pub fn update_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else {
        "linux"
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
        "{}/v1/agent/updates?agent_id={}&current_version={}&channel={}&os={}&arch={}",
        base.trim_end_matches('/'),
        agent_id,
        env!("CARGO_PKG_VERSION"),
        config.updates.channel,
        update_os(),
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
    // Asking for the running platform and *checking* the answer are both
    // necessary: a Linux artifact installed on a Windows host would pass the
    // signature and hash and then fail its self-test, which is a confusing way to
    // discover a mis-published release.
    anyhow::ensure!(
        manifest.os == update_os(),
        "update is for {} but this Agent runs on {}",
        manifest.os,
        update_os()
    );
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
    activate_installed_binary(&pending.manifest.version);
    Ok(Some(pending.manifest.version))
}

/// Starts running the binary that was just installed.
///
/// On Linux the old process keeps running from the replaced inode until the
/// service is restarted, which systemd's update path unit or the init script does
/// at the next start. Windows has no equivalent: the swap succeeded because a
/// running executable can be renamed, but this process is still executing the old
/// file and nothing will replace it on its own. So the restart is requested here.
fn activate_installed_binary(version: &str) {
    #[cfg(windows)]
    {
        use crate::windows_service;
        if windows_service::started_by_service_manager() {
            // A service cannot stop and start itself - the SCM refuses a start
            // for a service that is still stopping - so it stops with the
            // recovery-triggering exit code the installer configured, and the SCM
            // brings it back on the new binary.
            info!(
                %version,
                "update installed; stopping so the service manager restarts on the new binary"
            );
            windows_service::request_restart();
            return;
        }
        // Run from a console: restart the installed service, if there is one, so
        // the operator is not left with a new binary and an old process.
        match windows_service::restart_service_externally() {
            Ok(()) => info!(%version, "update installed and the service was restarted"),
            Err(error) => info!(
                %version,
                reason = %format!("{error:#}"),
                "update installed; restart the service to run it"
            ),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = version;
    }
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
    crate::platform::create_private_dir(&directory).context("create update staging directory")?;
    // The self-test executes this file, and Windows decides what is executable by
    // extension: staged without .exe it cannot be run, so the test that protects
    // the fleet would fail on every Windows host for the wrong reason.
    let artifact = directory.join(format!(
        "invenqor-agent-{}{}",
        manifest.version,
        std::env::consts::EXE_SUFFIX
    ));
    atomic_write(&artifact, bytes)?;
    crate::platform::make_executable(&artifact)?;
    let pending = serde_json::to_vec(&PendingUpdate { manifest, artifact })?;
    atomic_write(&directory.join("pending.json"), &pending)
}

fn atomic_install(target: &Path, bytes: &[u8], version: &str) -> Result<()> {
    let parent = target
        .parent()
        .context("update install path has no parent")?;
    // The candidate is written beside the target, not in the staging directory:
    // the swap below must be a rename within one volume, and %ProgramData% and
    // %ProgramFiles% are not guaranteed to be the same one. The extension matters
    // on Windows, where the self-test cannot execute a file without it.
    let temporary = parent.join(format!(
        ".invenqor-agent.update-{}{}",
        std::process::id(),
        std::env::consts::EXE_SUFFIX
    ));
    atomic_write(&temporary, bytes)?;
    crate::platform::make_executable(&temporary)?;
    // Run the candidate before it becomes the installed agent. A signed,
    // correctly hashed binary can still be unable to start - wrong architecture
    // family, a missing kernel feature, a bad build - and activating it would
    // stop collection on every host that received it, with no way back except
    // hands on each machine.
    if let Err(error) = self_test(&temporary, version) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    // Windows will not let anything delete or overwrite a running executable, but
    // it will let it be *renamed*: the running process keeps executing from the
    // moved file. So the old binary is moved aside first and the new one takes the
    // original path, which is the only sequence that works while the service is
    // running - and it happens to be the same sequence Linux wants anyway.
    let previous = target.with_extension("previous");
    if target.exists() {
        let _ = fs::remove_file(&previous);
        replace_file(target, &previous).context("preserve previous agent binary")?;
    }
    if let Err(error) = replace_file(&temporary, target) {
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

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut file = crate::platform::create_private_file(&temporary)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    drop(file);
    replace_file(&temporary, path)?;
    sync_directory(path.parent().context("path has no parent")?)
}

/// Flushes the directory entry so a rename survives a power loss. Windows has no
/// equivalent call for a directory handle opened this way, and NTFS metadata
/// journaling covers the same ground, so it is a no-op there.
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Moves `from` onto `to`, retrying briefly.
///
/// On Windows a file that was just written is routinely held open for a moment by
/// a virus scanner, and the rename fails with a sharing violation. Failing the
/// update for that would leave a host on the old version until someone noticed,
/// so the rename is retried for a few seconds before it is called an error.
fn replace_file(from: &Path, to: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..20 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                if attempt < 19 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
    }
    Err(last.expect("at least one attempt")).with_context(|| {
        format!(
            "replace {} - the file is held open by another process",
            to.display()
        )
    })
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
        let script = if cfg!(windows) {
            format!("@echo off\r\necho invenqor-agent {version}\r\nexit /b {exit_code}\r\n")
        } else {
            format!("#!/bin/sh\necho \"invenqor-agent {version}\"\nexit {exit_code}\n")
        };
        atomic_write(path, script.as_bytes()).unwrap();
        crate::platform::make_executable(path).unwrap();
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

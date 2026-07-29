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
    loop {
        match check_and_stage(&config, &agent_id).await {
            Ok(Some(version)) => info!(%version, "signed agent update staged"),
            Ok(None) => {}
            Err(error) => warn!(error = %error, "agent update check failed"),
        }
        tokio::time::sleep(Duration::from_secs(config.updates.check_interval_seconds)).await;
    }
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
    anyhow::ensure!(
        is_newer(env!("CARGO_PKG_VERSION"), &manifest.version),
        "server offered a non-newer update"
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
    atomic_install(&config.updates.install_path, &bytes)?;
    fs::remove_file(&pending_path).context("remove applied update marker")?;
    Ok(Some(pending.manifest.version))
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

fn atomic_install(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target
        .parent()
        .context("update install path has no parent")?;
    let temporary = parent.join(format!(".invenqor-agent.update-{}", std::process::id()));
    atomic_write(&temporary, bytes, 0o755)?;
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

    #[test]
    fn verifies_signature_and_applies_atomically_with_backup() {
        let root = tempdir().unwrap();
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let bytes = b"new-agent-binary";
        let manifest = UpdateManifest {
            version: "9.0.0".into(),
            channel: "stable".into(),
            os: "linux".into(),
            architecture: std::env::consts::ARCH.into(),
            sha256: hex::encode(Sha256::digest(bytes)),
            signature: STANDARD.encode(signing.sign(bytes).to_bytes()),
            download_url: "/artifact".into(),
            size: bytes.len() as u64,
        };
        verify_artifact(
            bytes,
            &manifest,
            &STANDARD.encode(signing.verifying_key().to_bytes()),
        )
        .unwrap();
        let target = root.path().join("agent");
        fs::write(&target, b"old").unwrap();
        atomic_install(&target, bytes).unwrap();
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert_eq!(fs::read(target.with_extension("previous")).unwrap(), b"old");
    }

    #[test]
    fn rejects_tampering_and_downgrades() {
        assert!(is_newer("1.2.3", "1.2.4"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("2.0.0", "1.9.9"));
    }
}

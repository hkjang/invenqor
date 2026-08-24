use crate::config::Config;
use crate::storage::StateStore;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use reqwest::{Certificate, Client, Identity, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tracing::{info, warn};

const MAX_UPDATE_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_PENDING_MARKER_BYTES: usize = 64 * 1024;
const MAX_UPDATE_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SELF_TEST_STREAM_BYTES: usize = 64 * 1024;
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNATURE_SCHEME_ED25519: &str = "ed25519";
const SIGNATURE_VERSION_LEGACY: u8 = 1;
const SIGNATURE_VERSION_V2: u8 = 2;
const SIGNATURE_DOMAIN_V2: &str = "INVENQOR-AGENT-UPDATE-MANIFEST-V2";

fn default_signature_scheme() -> String {
    SIGNATURE_SCHEME_ED25519.to_string()
}

fn default_signature_version() -> u8 {
    SIGNATURE_VERSION_LEGACY
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub channel: String,
    pub os: String,
    pub architecture: String,
    pub sha256: String,
    /// Artifact-only Ed25519 signature retained on v2 releases so Agents older
    /// than the manifest-signature protocol can consume a bridge upgrade.
    pub signature: String,
    /// Ed25519 signature over the canonical v2 metadata contract. Absent on
    /// stored legacy manifests.
    #[serde(default)]
    pub manifest_signature: String,
    /// Legacy manifests omitted both fields and signed only the artifact bytes.
    /// New releases sign the security-sensitive manifest metadata as v2.
    #[serde(default = "default_signature_scheme")]
    pub signature_scheme: String,
    #[serde(default = "default_signature_version")]
    pub signature_version: u8,
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
            Ok(Some(version)) => {
                info!(%version, "signed agent update staged");
                // Linux packages install a root-owned systemd path helper that
                // notices pending.json and performs the privileged swap. The
                // Windows service already runs as LocalSystem and has no path
                // helper, so staging without applying leaves every Windows host
                // on the old version forever. Complete the verified swap in the
                // service process; apply_pending requests an SCM restart only
                // after the candidate has passed its version self-test.
                match apply_staged_in_process(|| apply_pending(&config)) {
                    Ok(Some(installed)) => {
                        info!(version = %installed, "signed agent update installed automatically")
                    }
                    Ok(None) => {}
                    Err(error) => warn!(
                        version = %version,
                        error = %format!("{error:#}"),
                        "verified agent update is staged but could not be installed"
                    ),
                }
            }
            Ok(None) => {}
            Err(error) => warn!(error = %format!("{error:#}"), "agent update check failed"),
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

/// Windows has no privileged path watcher: the installed service is itself the
/// privileged updater. Linux deliberately returns None and leaves the marker for
/// the root-owned systemd helper (or the init script at restart).
fn apply_staged_in_process<F>(apply: F) -> Result<Option<String>>
where
    F: FnOnce() -> Result<Option<String>>,
{
    if cfg!(windows) {
        apply()
    } else {
        Ok(None)
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
    let manifest_body =
        read_bounded_response(response, MAX_UPDATE_MANIFEST_BYTES, "update manifest").await?;
    let manifest: UpdateManifest =
        serde_json::from_slice(&manifest_body).context("decode update manifest")?;
    let current = env!("CARGO_PKG_VERSION");
    validate_manifest_offer(
        &manifest,
        &config.updates.channel,
        current,
        update_os(),
        std::env::consts::ARCH,
    )?;
    let download = format!("{}{}", base.trim_end_matches('/'), manifest.download_url);
    let mut request = client.get(download);
    if let Some(token) = &bearer_token {
        request = request.bearer_auth(token);
    }
    let mut response = request.send().await.context("download update artifact")?;
    anyhow::ensure!(
        response.status().is_success(),
        "update download returned {}",
        response.status()
    );
    validate_declared_download_size(response.content_length(), manifest.size)?;

    // Proxies and Ingress controllers commonly use chunked transfer encoding,
    // where Content-Length is legitimately absent. Stream into a manifest-sized
    // buffer instead of using Response::bytes(): that accepts chunked responses
    // without allowing a malicious or broken peer to make an unbounded
    // allocation. The signed manifest itself is capped at 128 MiB above.
    let expected_size = usize::try_from(manifest.size).context("update size is too large")?;
    let mut bytes = Vec::with_capacity(expected_size);
    while let Some(chunk) = response.chunk().await.context("read update artifact")? {
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= expected_size,
            "update size exceeds manifest"
        );
        bytes.extend_from_slice(&chunk);
    }
    anyhow::ensure!(bytes.len() == expected_size, "update size does not match");
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

fn validate_declared_download_size(content_length: Option<u64>, expected: u64) -> Result<()> {
    if let Some(content_length) = content_length {
        anyhow::ensure!(
            content_length == expected,
            "update response Content-Length does not match manifest"
        );
    }
    Ok(())
}

async fn read_bounded_response(
    mut response: reqwest::Response,
    limit: usize,
    description: &str,
) -> Result<Vec<u8>> {
    if let Some(length) = response.content_length() {
        anyhow::ensure!(
            length <= limit as u64,
            "{description} response exceeds the {limit}-byte limit"
        );
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0)
            .min(limit),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("read {description} response"))?
    {
        anyhow::ensure!(
            body.len().saturating_add(chunk.len()) <= limit,
            "{description} response exceeds the {limit}-byte limit"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Validates every piece of unsigned routing metadata before any artifact is
/// downloaded. The artifact signature proves the bytes; this check proves those
/// bytes were offered to the channel and platform this Agent actually requested.
fn validate_manifest_offer(
    manifest: &UpdateManifest,
    requested_channel: &str,
    current: &str,
    os: &str,
    architecture: &str,
) -> Result<()> {
    anyhow::ensure!(
        parse_version(&manifest.version).is_some(),
        "update version must be three decimal numbers"
    );
    anyhow::ensure!(
        manifest.signature_scheme == SIGNATURE_SCHEME_ED25519,
        "unsupported update signature scheme"
    );
    anyhow::ensure!(
        matches!(
            manifest.signature_version,
            SIGNATURE_VERSION_LEGACY | SIGNATURE_VERSION_V2
        ),
        "unsupported update signature version"
    );
    anyhow::ensure!(
        !(manifest.allow_downgrade && manifest.signature_version == SIGNATURE_VERSION_LEGACY),
        "legacy artifact-only signatures cannot authorize a rollback"
    );
    anyhow::ensure!(
        is_newer(current, &manifest.version)
            || (manifest.allow_downgrade && manifest.version != current),
        "server offered {} which is not newer than {current} and is not marked as a rollback",
        manifest.version
    );
    anyhow::ensure!(
        manifest.channel == requested_channel,
        "update is for channel {} but this Agent requested {requested_channel}",
        manifest.channel
    );
    // Asking for the running platform and *checking* the answer are both
    // necessary: a Linux artifact installed on a Windows host would pass the
    // signature and hash and then fail its self-test, which is a confusing way to
    // discover a mis-published release.
    anyhow::ensure!(
        manifest.os == os,
        "update is for {} but this Agent runs on {}",
        manifest.os,
        os
    );
    anyhow::ensure!(
        manifest.architecture == architecture,
        "update architecture does not match"
    );
    let expected_download_url = format!(
        "/v1/agent/updates/{}-{}-{}/artifact",
        manifest.version, manifest.os, manifest.architecture
    );
    anyhow::ensure!(
        manifest.download_url == expected_download_url,
        "update download URL does not match the signed release identity"
    );
    anyhow::ensure!(
        manifest.size > 0 && manifest.size <= MAX_UPDATE_ARTIFACT_BYTES,
        "update manifest size is invalid"
    );
    Ok(())
}

/// A privileged pending-update apply must never follow a path selected by the
/// unprivileged Agent. The directory is canonicalized once and, on Unix, all
/// file operations are anchored to an open directory descriptor. That closes
/// the rename/symlink race between checking a path and opening or removing it.
struct TrustedUpdatesDirectory {
    path: PathBuf,
    #[cfg(unix)]
    handle: File,
}

impl TrustedUpdatesDirectory {
    fn open(state_dir: &Path) -> Result<Option<Self>> {
        let state_dir = match fs::canonicalize(state_dir) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("canonicalize agent state directory"),
        };
        anyhow::ensure!(
            fs::metadata(&state_dir)?.is_dir(),
            "agent state path is not a directory"
        );
        let updates_path = state_dir.join("updates");
        let metadata = match fs::symlink_metadata(&updates_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect update staging directory"),
        };
        anyhow::ensure!(
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
            "update staging path must be a real directory, not a symlink"
        );
        let canonical =
            fs::canonicalize(&updates_path).context("canonicalize update staging directory")?;
        anyhow::ensure!(
            canonical == updates_path,
            "update staging directory escaped the canonical state directory"
        );

        #[cfg(unix)]
        {
            let handle = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&canonical)
                .context("open trusted update staging directory")?;
            anyhow::ensure!(
                handle.metadata()?.is_dir(),
                "update staging path is not a directory"
            );
            Ok(Some(Self {
                path: canonical,
                handle,
            }))
        }
        #[cfg(not(unix))]
        {
            Ok(Some(Self { path: canonical }))
        }
    }

    fn read_regular_bounded(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        validate_staged_filename(name)?;
        #[cfg(unix)]
        let file = {
            let c_name = CString::new(name).context("update filename contains a NUL byte")?;
            let descriptor = unsafe {
                libc::openat(
                    self.handle.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    0,
                )
            };
            if descriptor < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(error).with_context(|| {
                    format!("open staged update {}", self.path.join(name).display())
                });
            }
            // SAFETY: openat returned a new descriptor owned by this call.
            unsafe { File::from_raw_fd(descriptor) }
        };
        #[cfg(not(unix))]
        let file = {
            let path = self.path.join(name);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error).context("inspect staged update"),
            };
            anyhow::ensure!(
                metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
                "staged update must be a regular file, not a symlink"
            );
            OpenOptions::new()
                .read(true)
                .open(&path)
                .context("open staged update")?
        };

        let metadata = file.metadata().context("inspect open staged update")?;
        anyhow::ensure!(
            metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
            "staged update must be a regular file, not a symlink"
        );
        anyhow::ensure!(
            metadata.len() <= limit as u64,
            "staged update exceeds the {limit}-byte limit"
        );
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .context("read staged update")?;
        anyhow::ensure!(
            bytes.len() <= limit,
            "staged update exceeds the {limit}-byte limit"
        );
        Ok(Some(bytes))
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        validate_staged_filename(name).map_err(std::io::Error::other)?;
        #[cfg(unix)]
        {
            let name = CString::new(name)
                .map_err(|_| std::io::Error::other("update filename contains a NUL byte"))?;
            if unsafe { libc::unlinkat(self.handle.as_raw_fd(), name.as_ptr(), 0) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
        #[cfg(not(unix))]
        {
            fs::remove_file(self.path.join(name))
        }
    }
}

fn validate_staged_filename(name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && !name.contains('/')
            && !name.contains('\\'),
        "invalid staged update filename"
    );
    Ok(())
}

fn staged_artifact_name(version: &str) -> Result<String> {
    anyhow::ensure!(
        parse_version(version).is_some(),
        "update version must be three decimal numbers"
    );
    Ok(format!(
        "invenqor-agent-{version}{}",
        std::env::consts::EXE_SUFFIX
    ))
}

pub fn apply_pending(config: &Config) -> Result<Option<String>> {
    let Some(directory) = TrustedUpdatesDirectory::open(&config.agent.state_dir)? else {
        return Ok(None);
    };
    let Some(marker) = directory.read_regular_bounded("pending.json", MAX_PENDING_MARKER_BYTES)?
    else {
        return Ok(None);
    };
    let pending: PendingUpdate =
        serde_json::from_slice(&marker).context("decode pending update")?;
    validate_manifest_offer(
        &pending.manifest,
        &config.updates.channel,
        env!("CARGO_PKG_VERSION"),
        update_os(),
        std::env::consts::ARCH,
    )?;
    let artifact_name = staged_artifact_name(&pending.manifest.version)?;
    let expected_size =
        usize::try_from(pending.manifest.size).context("pending update size is too large")?;
    let bytes = directory
        .read_regular_bounded(&artifact_name, expected_size)?
        .context("staged update is missing")?;
    anyhow::ensure!(
        bytes.len() == expected_size,
        "staged update size does not match manifest"
    );
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
    directory
        .remove("pending.json")
        .context("remove applied update marker")?;
    // The staged copy is several megabytes and was previously left behind on
    // every update, so a long-lived host slowly filled its state directory.
    if let Err(error) = directory.remove(&artifact_name) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warn!(
                artifact = %directory.path.join(&artifact_name).display(),
                error = %error,
                "could not remove the staged update artifact"
            );
        }
    }
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
            // for a service that is still stopping. The service callback exits
            // without a clean SERVICE_STOPPED notification, so SCM recovery also
            // works before failureflag becomes effective after the first reboot.
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
    let Ok(applied_name) = staged_artifact_name(applied_version) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("invenqor-agent-") {
            continue;
        }
        if name == applied_name {
            continue;
        }
        let _ = fs::remove_file(entry.path());
    }
}

fn verify_artifact(bytes: &[u8], manifest: &UpdateManifest, public_key: &str) -> Result<()> {
    anyhow::ensure!(
        bytes.len() as u64 == manifest.size,
        "update size does not match manifest"
    );
    let digest = hex::encode(Sha256::digest(bytes));
    anyhow::ensure!(
        digest == manifest.sha256.to_ascii_lowercase(),
        "update SHA-256 mismatch"
    );
    let key: [u8; 32] = STANDARD
        .decode(public_key.trim())
        .context("decode update public key")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Ed25519 public key must be 32 bytes"))?;
    anyhow::ensure!(
        manifest.signature_scheme == SIGNATURE_SCHEME_ED25519,
        "unsupported update signature scheme"
    );
    let verifying_key = VerifyingKey::from_bytes(&key).context("parse update public key")?;
    match manifest.signature_version {
        SIGNATURE_VERSION_LEGACY => {
            anyhow::ensure!(
                !manifest.allow_downgrade,
                "legacy artifact-only signatures cannot authorize a rollback"
            );
            verify_ed25519_signature(
                &verifying_key,
                bytes,
                &manifest.signature,
                "legacy artifact",
            )
        }
        SIGNATURE_VERSION_V2 => {
            // Verify the bridge signature too: old Agents use this field, while
            // this Agent uses manifest_signature to authenticate rollback and
            // routing metadata. Requiring both prevents publishing a release
            // that silently strands the pre-v2 fleet.
            verify_ed25519_signature(
                &verifying_key,
                bytes,
                &manifest.signature,
                "legacy bridge artifact",
            )?;
            verify_ed25519_signature(
                &verifying_key,
                &signature_message_v2(manifest)?,
                &manifest.manifest_signature,
                "v2 manifest",
            )
        }
        _ => anyhow::bail!("unsupported update signature version"),
    }
}

fn verify_ed25519_signature(
    verifying_key: &VerifyingKey,
    message: &[u8],
    encoded_signature: &str,
    description: &str,
) -> Result<()> {
    let signature = Signature::from_slice(
        &STANDARD
            .decode(encoded_signature.trim())
            .with_context(|| format!("decode {description} signature"))?,
    )
    .with_context(|| format!("parse {description} signature"))?;
    verifying_key
        .verify(message, &signature)
        .with_context(|| format!("verify {description} signature"))
}

/// Canonical Ed25519 v2 contract shared with the Server and offline signer.
/// The executable is transitively bound by its exact size and SHA-256 digest,
/// which `verify_artifact` checks before this signature is accepted.
fn signature_message_v2(manifest: &UpdateManifest) -> Result<Vec<u8>> {
    anyhow::ensure!(
        manifest.signature_scheme == SIGNATURE_SCHEME_ED25519
            && manifest.signature_version == SIGNATURE_VERSION_V2,
        "manifest is not an Ed25519 v2 signature contract"
    );
    anyhow::ensure!(manifest.size > 0, "manifest size must be positive");
    anyhow::ensure!(
        manifest.sha256.len() == 64
            && manifest.sha256 == manifest.sha256.to_ascii_lowercase()
            && hex::decode(&manifest.sha256).is_ok(),
        "manifest SHA-256 must be 64 lowercase hexadecimal characters"
    );
    for (field, value) in [
        ("version", manifest.version.as_str()),
        ("channel", manifest.channel.as_str()),
        ("os", manifest.os.as_str()),
        ("architecture", manifest.architecture.as_str()),
    ] {
        anyhow::ensure!(
            !value.is_empty() && !value.contains(['\r', '\n']),
            "manifest {field} is empty or contains a line break"
        );
    }
    Ok(format!(
        "{SIGNATURE_DOMAIN_V2}\nversion={}\nchannel={}\nos={}\narchitecture={}\nsize={}\nsha256={}\nallow_downgrade={}\n",
        manifest.version,
        manifest.channel,
        manifest.os,
        manifest.architecture,
        manifest.size,
        manifest.sha256,
        manifest.allow_downgrade,
    )
    .into_bytes())
}

fn stage(config: &Config, manifest: UpdateManifest, bytes: &[u8]) -> Result<()> {
    let directory = config.agent.state_dir.join("updates");
    crate::platform::create_private_dir(&directory).context("create update staging directory")?;
    // The self-test executes this file, and Windows decides what is executable by
    // extension: staged without .exe it cannot be run, so the test that protects
    // the fleet would fail on every Windows host for the wrong reason.
    let artifact = directory.join(staged_artifact_name(&manifest.version)?);
    atomic_write(&artifact, bytes)?;
    crate::platform::make_executable(&artifact)?;
    // Cleanup runs while the unprivileged Agent owns the operation. The
    // privileged helper never enumerates or deletes marker-selected paths.
    prune_staged_artifacts(config, &manifest.version);
    let pending = serde_json::to_vec(&PendingUpdate { manifest })?;
    atomic_write(&directory.join("pending.json"), &pending)
}

fn atomic_install(target: &Path, bytes: &[u8], version: &str) -> Result<()> {
    atomic_install_with_suffix(target, bytes, version, std::env::consts::EXE_SUFFIX)
}

/// Installs an artifact using the platform's executable suffix for the
/// pre-activation self-test. Keeping the suffix explicit also lets the tests
/// exercise the real command execution and rename sequence with a `.cmd`
/// fixture on Windows instead of pretending batch-file bytes are a PE image.
fn atomic_install_with_suffix(
    target: &Path,
    bytes: &[u8],
    version: &str,
    executable_suffix: &str,
) -> Result<()> {
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
        executable_suffix
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
    // Append instead of replacing the extension. On Windows, `with_extension`
    // turns `invenqor-agent.exe` into `invenqor-agent.previous`, while the
    // installer, uninstaller and documented rollback contract all use
    // `invenqor-agent.exe.previous`.
    let previous = previous_binary_path(target);
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

fn previous_binary_path(target: &Path) -> PathBuf {
    let mut path = target.as_os_str().to_os_string();
    path.push(".previous");
    PathBuf::from(path)
}

/// Verifies that the staged binary actually runs and reports the version the
/// manifest promised.
fn self_test(candidate: &Path, expected_version: &str) -> Result<()> {
    self_test_with_limits(
        candidate,
        expected_version,
        SELF_TEST_TIMEOUT,
        MAX_SELF_TEST_STREAM_BYTES,
    )
}

#[derive(Debug)]
struct BoundedStream {
    bytes: Vec<u8>,
    truncated: bool,
}

/// Owns the OS mechanism used to terminate a self-test and any descendants that
/// inherited its output pipes. Unix uses the process group created before exec;
/// Windows assigns the candidate to a Job Object, whose explicit termination
/// reaches child processes as well as the direct candidate.
struct SelfTestKillScope {
    #[cfg(unix)]
    process_group: i32,
    #[cfg(windows)]
    job: isize,
}

impl SelfTestKillScope {
    fn new(child: &std::process::Child) -> Self {
        #[cfg(unix)]
        {
            Self {
                process_group: child.id() as i32,
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job != 0
                && unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as isize) } == 0
            {
                unsafe { CloseHandle(job) };
                warn!(
                    error = %std::io::Error::last_os_error(),
                    "could not place the update self-test in a Windows Job Object; descendant cleanup is best effort"
                );
                return Self { job: 0 };
            }
            Self { job }
        }
    }

    fn terminate(&self, child: &mut std::process::Child) {
        #[cfg(unix)]
        unsafe {
            // SAFETY: CommandExt::process_group placed the candidate in a fresh
            // group whose id is the direct child's pid.
            libc::kill(-self.process_group, libc::SIGKILL);
        }
        #[cfg(windows)]
        if self.job != 0 {
            unsafe { TerminateJobObject(self.job, 1) };
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(windows)]
impl Drop for SelfTestKillScope {
    fn drop(&mut self) {
        if self.job != 0 {
            unsafe { CloseHandle(self.job) };
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(attributes: *const std::ffi::c_void, name: *const u16) -> isize;
    fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
    fn TerminateJobObject(job: isize, exit_code: u32) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

fn read_stream_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<BoundedStream> {
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    let mut truncated = false;
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        let retained = remaining.min(count);
        bytes.extend_from_slice(&chunk[..retained]);
        truncated |= retained != count;
        // Keep draining after the cap. Otherwise a candidate that writes a lot
        // can block forever on a full pipe before the timeout logic can reap it.
    }
    Ok(BoundedStream { bytes, truncated })
}

fn self_test_with_limits(
    candidate: &Path,
    expected_version: &str,
    timeout: Duration,
    stream_limit: usize,
) -> Result<()> {
    let mut command = Command::new(candidate);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A broken candidate may spawn a child that inherits its pipes. A
        // dedicated process group lets the deadline reap that whole self-test.
        command.process_group(0);
    }
    let mut child = command.spawn().with_context(|| {
        format!(
            "the staged agent {} could not be executed, so it was not installed",
            candidate.display()
        )
    })?;
    let kill_scope = SelfTestKillScope::new(&child);
    let Some(stdout) = child.stdout.take() else {
        kill_scope.terminate(&mut child);
        anyhow::bail!("capture staged agent stdout");
    };
    let Some(stderr) = child.stderr.take() else {
        kill_scope.terminate(&mut child);
        anyhow::bail!("capture staged agent stderr");
    };
    let (stdout_tx, stdout_rx) = std::sync::mpsc::sync_channel(1);
    let (stderr_tx, stderr_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = stdout_tx.send(read_stream_bounded(stdout, stream_limit));
    });
    std::thread::spawn(move || {
        let _ = stderr_tx.send(read_stream_bounded(stderr, stream_limit));
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                kill_scope.terminate(&mut child);
                return Err(error).context("poll staged agent self-test");
            }
        }
        if Instant::now() >= deadline {
            kill_scope.terminate(&mut child);
            anyhow::bail!(
                "the staged agent exceeded its {} second self-test deadline, so it was killed and not installed",
                timeout.as_secs_f64()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let receive = |receiver: std::sync::mpsc::Receiver<std::io::Result<BoundedStream>>,
                   description: &str|
     -> Result<BoundedStream> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        anyhow::ensure!(
            !remaining.is_zero(),
            "the staged agent did not close {description} before the self-test deadline"
        );
        receiver
            .recv_timeout(remaining)
            .with_context(|| {
                format!(
                    "the staged agent did not close {description} before the self-test deadline"
                )
            })?
            .with_context(|| format!("read staged agent {description}"))
    };
    let captured = (|| -> Result<(BoundedStream, BoundedStream)> {
        Ok((receive(stdout_rx, "stdout")?, receive(stderr_rx, "stderr")?))
    })();
    let (stdout, stderr) = match captured {
        Ok(captured) => captured,
        Err(error) => {
            // The direct process may already have exited while a descendant
            // keeps a pipe open. Reap the entire group/job on this path too.
            kill_scope.terminate(&mut child);
            return Err(error);
        }
    };
    anyhow::ensure!(
        !stdout.truncated,
        "the staged agent wrote more than {stream_limit} bytes to stdout during its self-test"
    );
    anyhow::ensure!(
        !stderr.truncated,
        "the staged agent wrote more than {stream_limit} bytes to stderr during its self-test"
    );
    anyhow::ensure!(
        status.success(),
        "the staged agent exited with {} during its self-test, so it was not installed: {}",
        status,
        String::from_utf8_lossy(&stderr.bytes).trim()
    );
    let reported = String::from_utf8_lossy(&stdout.bytes);
    let expected = format!("invenqor-agent {expected_version}");
    anyhow::ensure!(
        reported.trim() == expected,
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
        match rename_replacing(from, to) {
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

#[cfg(not(windows))]
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let wide = |path: &Path| -> std::io::Result<Vec<u16>> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "update path contains a NUL character",
            ));
        }
        value.push(0);
        Ok(value)
    };
    let existing = wide(from)?;
    let replacement = wide(to)?;
    if unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
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
    match (parse_version(current), parse_version(candidate)) {
        (Some(left), Some(right)) => right > left,
        _ => false,
    }
}

fn parse_version(value: &str) -> Option<[u64; 3]> {
    let mut pieces = value.split('.');
    let first = pieces.next()?;
    let second = pieces.next()?;
    let third = pieces.next()?;
    if pieces.next().is_some()
        || [first, second, third]
            .iter()
            .any(|piece| piece.is_empty() || !piece.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let parts = [
        first.parse().ok()?,
        second.parse().ok()?,
        third.parse().ok()?,
    ];
    // Match the Server's canonical release identifier and avoid two filenames
    // representing the same version (for example 01.2.3).
    (value == format!("{}.{}.{}", parts[0], parts[1], parts[2])).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use tempfile::tempdir;

    const FIXTURE_EXECUTABLE_SUFFIX: &str = if cfg!(windows) { ".cmd" } else { "" };

    #[test]
    fn backup_path_appends_previous_without_dropping_the_executable_suffix() {
        assert_eq!(
            previous_binary_path(Path::new("invenqor-agent.exe")),
            PathBuf::from("invenqor-agent.exe.previous")
        );
        assert_eq!(
            previous_binary_path(Path::new("invenqor-agent")),
            PathBuf::from("invenqor-agent.previous")
        );
    }

    #[test]
    fn atomic_write_replaces_an_existing_marker_or_staged_artifact() {
        let root = tempdir().unwrap();
        let destination = root.path().join("pending.json");
        atomic_write(&destination, b"first").unwrap();
        atomic_write(&destination, b"second").unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"second");
    }

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

    fn signed_manifest(
        bytes: &[u8],
        signing: &SigningKey,
        version: &str,
        signature_version: u8,
        allow_downgrade: bool,
    ) -> UpdateManifest {
        let mut manifest = UpdateManifest {
            version: version.into(),
            channel: "stable".into(),
            os: update_os().into(),
            architecture: std::env::consts::ARCH.into(),
            sha256: hex::encode(Sha256::digest(bytes)),
            signature: String::new(),
            manifest_signature: String::new(),
            signature_scheme: SIGNATURE_SCHEME_ED25519.into(),
            signature_version,
            download_url: format!(
                "/v1/agent/updates/{version}-{}-{}/artifact",
                update_os(),
                std::env::consts::ARCH
            ),
            size: bytes.len() as u64,
            allow_downgrade,
            notes: String::new(),
        };
        manifest.signature = STANDARD.encode(signing.sign(bytes).to_bytes());
        if signature_version == SIGNATURE_VERSION_V2 {
            manifest.manifest_signature = STANDARD.encode(
                signing
                    .sign(&signature_message_v2(&manifest).unwrap())
                    .to_bytes(),
            );
        }
        manifest
    }

    fn read_headers(stream: &mut TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "client closed before sending HTTP headers");
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|value| value == b"\r\n\r\n") {
                return;
            }
        }
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
            manifest_signature: String::new(),
            signature_scheme: SIGNATURE_SCHEME_ED25519.into(),
            signature_version: SIGNATURE_VERSION_LEGACY,
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
        fs::write(previous_binary_path(&target), b"stale backup").unwrap();
        atomic_install_with_suffix(&target, &bytes, "9.0.0", FIXTURE_EXECUTABLE_SUFFIX).unwrap();
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert_eq!(fs::read(previous_binary_path(&target)).unwrap(), b"old");
    }

    #[test]
    fn v2_signature_binds_every_security_sensitive_manifest_field() {
        let signing = SigningKey::from_bytes(&[17u8; 32]);
        let bytes = b"signed v2 artifact";
        let manifest = signed_manifest(bytes, &signing, "9.0.0", SIGNATURE_VERSION_V2, false);
        let public_key = format!(
            "  {}\n",
            STANDARD.encode(signing.verifying_key().to_bytes())
        );
        verify_artifact(bytes, &manifest, &public_key).unwrap();

        for mutate in [
            |value: &mut UpdateManifest| value.version = "9.0.1".into(),
            |value: &mut UpdateManifest| value.channel = "beta".into(),
            |value: &mut UpdateManifest| value.os = "other".into(),
            |value: &mut UpdateManifest| value.architecture = "other".into(),
            |value: &mut UpdateManifest| value.allow_downgrade = true,
        ] {
            let mut tampered = manifest.clone();
            mutate(&mut tampered);
            assert!(verify_artifact(bytes, &tampered, &public_key).is_err());
        }
        let mut tampered_size = manifest.clone();
        tampered_size.size += 1;
        assert!(verify_artifact(bytes, &tampered_size, &public_key).is_err());
        let mut tampered_hash = manifest;
        tampered_hash.sha256 = "00".repeat(32);
        assert!(verify_artifact(bytes, &tampered_hash, &public_key).is_err());
    }

    #[test]
    fn v2_bundle_preserves_the_v0_2_14_artifact_signature_contract() {
        #[derive(Deserialize)]
        struct LegacyWireManifest {
            sha256: String,
            signature: String,
            size: u64,
        }

        let signing = SigningKey::from_bytes(&[21u8; 32]);
        let bytes = b"bridge release artifact";
        let manifest = signed_manifest(bytes, &signing, "9.0.0", SIGNATURE_VERSION_V2, false);
        // serde's default unknown-field behavior matches the v0.2.14 Agent: it
        // ignores manifest_signature/signature_version and reads signature.
        let legacy: LegacyWireManifest =
            serde_json::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(legacy.size, bytes.len() as u64);
        assert_eq!(legacy.sha256, hex::encode(Sha256::digest(bytes)));
        let signature = Signature::from_slice(&STANDARD.decode(legacy.signature).unwrap()).unwrap();
        signing
            .verifying_key()
            .verify(bytes, &signature)
            .expect("v0.2.14 artifact-only verification must accept the bridge release");
        assert_ne!(manifest.signature, manifest.manifest_signature);
    }

    #[test]
    fn v2_canonical_message_matches_the_server_and_offline_signer_contract() {
        let manifest = UpdateManifest {
            version: "1.2.3".into(),
            channel: "stable".into(),
            os: "linux".into(),
            architecture: "x86_64".into(),
            sha256: "ab".repeat(32),
            signature: String::new(),
            manifest_signature: String::new(),
            signature_scheme: SIGNATURE_SCHEME_ED25519.into(),
            signature_version: SIGNATURE_VERSION_V2,
            download_url: "/v1/agent/updates/1.2.3-linux-x86_64/artifact".into(),
            size: 42,
            allow_downgrade: true,
            notes: String::new(),
        };
        assert_eq!(
            signature_message_v2(&manifest).unwrap(),
            format!(
                "INVENQOR-AGENT-UPDATE-MANIFEST-V2\nversion=1.2.3\nchannel=stable\nos=linux\narchitecture=x86_64\nsize=42\nsha256={}\nallow_downgrade=true\n",
                "ab".repeat(32)
            )
            .into_bytes()
        );
    }

    #[test]
    fn legacy_signature_allows_a_normal_upgrade_but_never_a_rollback() {
        let signing = SigningKey::from_bytes(&[18u8; 32]);
        let bytes = b"legacy artifact";
        let public_key = STANDARD.encode(signing.verifying_key().to_bytes());
        let upgrade = signed_manifest(bytes, &signing, "9.0.0", SIGNATURE_VERSION_LEGACY, false);
        verify_artifact(bytes, &upgrade, &public_key).unwrap();

        let rollback = signed_manifest(bytes, &signing, "0.1.0", SIGNATURE_VERSION_LEGACY, true);
        let error = verify_artifact(bytes, &rollback, &public_key).unwrap_err();
        assert!(format!("{error:#}").contains("cannot authorize a rollback"));
        assert!(validate_manifest_offer(
            &rollback,
            "stable",
            env!("CARGO_PKG_VERSION"),
            update_os(),
            std::env::consts::ARCH,
        )
        .is_err());
    }

    #[test]
    fn missing_signature_contract_fields_decode_as_legacy() {
        let decoded: UpdateManifest = serde_json::from_value(serde_json::json!({
            "version": "9.0.0",
            "channel": "stable",
            "os": update_os(),
            "architecture": std::env::consts::ARCH,
            "sha256": "00".repeat(32),
            "signature": "unused",
            "download_url": format!(
                "/v1/agent/updates/9.0.0-{}-{}/artifact",
                update_os(),
                std::env::consts::ARCH
            ),
            "size": 1
        }))
        .unwrap();
        assert_eq!(decoded.signature_scheme, SIGNATURE_SCHEME_ED25519);
        assert_eq!(decoded.signature_version, SIGNATURE_VERSION_LEGACY);
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
        assert!(!previous_binary_path(&target).exists());
    }

    #[test]
    fn refuses_a_binary_that_reports_the_wrong_version() {
        let root = tempdir().unwrap();
        let staged = root.path().join("candidate");
        let bytes = executable(&staged, "1.0.0", 0);
        let target = root.path().join("agent");
        fs::write(&target, b"working-agent").unwrap();
        let error = atomic_install_with_suffix(&target, &bytes, "2.0.0", FIXTURE_EXECUTABLE_SUFFIX)
            .unwrap_err();
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
        assert!(
            atomic_install_with_suffix(&target, &bytes, "3.0.0", FIXTURE_EXECUTABLE_SUFFIX)
                .is_err()
        );
        assert_eq!(fs::read(&target).unwrap(), b"working-agent");
    }

    #[test]
    fn self_test_capture_is_bounded_per_stream() {
        let output = read_stream_bounded(std::io::Cursor::new(vec![b'x'; 257]), 64).unwrap();
        assert_eq!(output.bytes.len(), 64);
        assert!(output.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn self_test_kills_a_hung_candidate_at_its_deadline() {
        let root = tempdir().unwrap();
        let candidate = root.path().join("hung-agent");
        atomic_write(&candidate, b"#!/bin/sh\nexec sleep 30\n").unwrap();
        crate::platform::make_executable(&candidate).unwrap();
        let started = Instant::now();
        let error = self_test_with_limits(&candidate, "9.0.0", Duration::from_millis(150), 1024)
            .unwrap_err();
        assert!(format!("{error:#}").contains("deadline"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn self_test_kills_descendants_that_keep_output_pipes_open() {
        let root = tempdir().unwrap();
        let candidate = root.path().join("forking-agent");
        let child_pid = root.path().join("child.pid");
        let script = format!(
            "#!/bin/sh\nsleep 30 &\necho $! > '{}'\necho 'invenqor-agent 9.0.0'\nexit 0\n",
            child_pid.display()
        );
        atomic_write(&candidate, script.as_bytes()).unwrap();
        crate::platform::make_executable(&candidate).unwrap();
        let error = self_test_with_limits(&candidate, "9.0.0", Duration::from_millis(250), 1024)
            .unwrap_err();
        assert!(format!("{error:#}").contains("did not close stdout"));

        let pid: i32 = fs::read_to_string(child_pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut gone = false;
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            gone,
            "self-test descendant {pid} survived group termination"
        );
    }

    #[test]
    fn rejects_tampering_and_downgrades() {
        assert!(is_newer("1.2.3", "1.2.4"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("2.0.0", "1.9.9"));
    }

    #[test]
    fn refuses_an_update_offer_for_a_different_channel_or_platform() {
        let manifest = UpdateManifest {
            version: "9.0.0".into(),
            channel: "stable".into(),
            os: "windows".into(),
            architecture: "x86_64".into(),
            sha256: "00".repeat(32),
            signature: String::new(),
            manifest_signature: String::new(),
            signature_scheme: SIGNATURE_SCHEME_ED25519.into(),
            signature_version: SIGNATURE_VERSION_LEGACY,
            download_url: "/v1/agent/updates/9.0.0-windows-x86_64/artifact".into(),
            size: 1,
            allow_downgrade: false,
            notes: String::new(),
        };
        validate_manifest_offer(&manifest, "stable", "0.2.14", "windows", "x86_64").unwrap();

        let mut wrong = manifest.clone();
        wrong.channel = "beta".into();
        assert!(format!(
            "{:#}",
            validate_manifest_offer(&wrong, "stable", "0.2.14", "windows", "x86_64").unwrap_err()
        )
        .contains("channel"));

        wrong = manifest.clone();
        wrong.os = "linux".into();
        assert!(validate_manifest_offer(&wrong, "stable", "0.2.14", "windows", "x86_64").is_err());

        wrong = manifest;
        wrong.download_url = "//evil.example/update".into();
        assert!(validate_manifest_offer(&wrong, "stable", "0.2.14", "windows", "x86_64").is_err());
    }

    #[test]
    fn accepts_chunked_update_downloads_but_rejects_a_wrong_declared_size() {
        validate_declared_download_size(None, 42).unwrap();
        validate_declared_download_size(Some(42), 42).unwrap();
        assert!(validate_declared_download_size(Some(41), 42).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn pending_marker_cannot_select_or_delete_the_installed_binary() {
        let root = tempdir().unwrap();
        let signing = SigningKey::from_bytes(&[19u8; 32]);
        let source = root.path().join("source-agent");
        let bytes = executable(&source, "9.0.0", 0);
        let manifest = signed_manifest(&bytes, &signing, "9.0.0", SIGNATURE_VERSION_LEGACY, false);
        let target = root.path().join("installed-agent");
        fs::write(&target, b"old agent").unwrap();
        let mut config = Config::default();
        config.agent.state_dir = root.path().join("state");
        config.updates.channel = "stable".into();
        config.updates.install_path = target.clone();
        config.updates.public_key = Some(format!(
            " \n{}\n ",
            STANDARD.encode(signing.verifying_key().to_bytes())
        ));
        stage(&config, manifest, &bytes).unwrap();

        // This is the legacy exploit shape: old helpers trusted this path both
        // for the root-owned read and for post-install deletion.
        let marker_path = config.agent.state_dir.join("updates/pending.json");
        let mut marker: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        marker["artifact"] = serde_json::json!(target);
        fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();

        assert_eq!(apply_pending(&config).unwrap().as_deref(), Some("9.0.0"));
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert!(previous_binary_path(&target).exists());
        assert!(!marker_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn privileged_pending_apply_rejects_symlinks_and_oversized_markers() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let state = root.path().join("state");
        let updates = state.join("updates");
        fs::create_dir_all(&updates).unwrap();
        let mut config = Config::default();
        config.agent.state_dir = state;

        let external = root.path().join("external-marker");
        fs::write(&external, b"{}").unwrap();
        symlink(&external, updates.join("pending.json")).unwrap();
        assert!(apply_pending(&config).is_err());

        fs::remove_file(updates.join("pending.json")).unwrap();
        fs::write(
            updates.join("pending.json"),
            vec![b'x'; MAX_PENDING_MARKER_BYTES + 1],
        )
        .unwrap();
        let error = apply_pending(&config).unwrap_err();
        assert!(format!("{error:#}").contains("65536-byte limit"));
    }

    #[cfg(unix)]
    #[test]
    fn privileged_pending_apply_rejects_a_symlinked_artifact() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let signing = SigningKey::from_bytes(&[20u8; 32]);
        let source = root.path().join("external-agent");
        let bytes = executable(&source, "9.0.0", 0);
        let manifest = signed_manifest(&bytes, &signing, "9.0.0", SIGNATURE_VERSION_LEGACY, false);
        let mut config = Config::default();
        config.agent.state_dir = root.path().join("state");
        config.updates.install_path = root.path().join("installed-agent");
        config.updates.public_key = Some(STANDARD.encode(signing.verifying_key().to_bytes()));
        stage(&config, manifest, &bytes).unwrap();
        let artifact = config
            .agent
            .state_dir
            .join("updates")
            .join(staged_artifact_name("9.0.0").unwrap());
        fs::remove_file(&artifact).unwrap();
        symlink(&source, &artifact).unwrap();

        assert!(apply_pending(&config).is_err());
        assert_eq!(fs::read(source).unwrap(), bytes);
        assert!(!config.updates.install_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn privileged_pending_apply_rejects_a_symlinked_updates_directory() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let state = root.path().join("state");
        let external = root.path().join("external");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&external).unwrap();
        symlink(&external, state.join("updates")).unwrap();
        let mut config = Config::default();
        config.agent.state_dir = state;
        let error = apply_pending(&config).unwrap_err();
        assert!(format!("{error:#}").contains("not a symlink"));
    }

    #[tokio::test]
    async fn stages_a_signed_chunked_update_without_content_length() {
        let root = tempdir().unwrap();
        let signing = SigningKey::from_bytes(&[9u8; 32]);
        let artifact = b"signed chunked update artifact".to_vec();
        let manifest = UpdateManifest {
            version: "9.0.0".into(),
            channel: "stable".into(),
            os: update_os().into(),
            architecture: std::env::consts::ARCH.into(),
            sha256: hex::encode(Sha256::digest(&artifact)),
            signature: STANDARD.encode(signing.sign(&artifact).to_bytes()),
            manifest_signature: String::new(),
            signature_scheme: SIGNATURE_SCHEME_ED25519.into(),
            signature_version: SIGNATURE_VERSION_LEGACY,
            download_url: format!(
                "/v1/agent/updates/9.0.0-{}-{}/artifact",
                update_os(),
                std::env::consts::ARCH
            ),
            size: artifact.len() as u64,
            allow_downgrade: false,
            notes: String::new(),
        };
        let manifest_body = serde_json::to_vec(&manifest).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_headers(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                manifest_body.len()
            )
            .unwrap();
            stream.write_all(&manifest_body).unwrap();

            let (mut stream, _) = listener.accept().unwrap();
            read_headers(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            for chunk in artifact.chunks(7) {
                write!(stream, "{:x}\r\n", chunk.len()).unwrap();
                stream.write_all(chunk).unwrap();
                stream.write_all(b"\r\n").unwrap();
            }
            stream.write_all(b"0\r\n\r\n").unwrap();
        });

        let mut config = Config::default();
        config.server.url = Some(format!("http://{address}"));
        config.server.allow_insecure_http = true;
        config.server.timeout_seconds = 5;
        config.agent.state_dir = root.path().join("state");
        config.updates.enabled = true;
        config.updates.public_key = Some(STANDARD.encode(signing.verifying_key().to_bytes()));
        config.updates.install_path = root.path().join("invenqor-agent");
        config.validate().unwrap();

        assert_eq!(
            check_and_stage(&config, "chunked-test-agent")
                .await
                .unwrap()
                .as_deref(),
            Some("9.0.0")
        );
        server.join().unwrap();
        assert!(config.agent.state_dir.join("updates/pending.json").exists());
    }

    #[tokio::test]
    async fn rejects_an_oversized_chunked_manifest_without_buffering_it_all() {
        let root = tempdir().unwrap();
        let signing = SigningKey::from_bytes(&[10u8; 32]);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_headers(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let chunk = vec![b'x'; 4096];
            for _ in 0..=(MAX_UPDATE_MANIFEST_BYTES / chunk.len()) {
                if write!(stream, "{:x}\r\n", chunk.len())
                    .and_then(|_| stream.write_all(&chunk))
                    .and_then(|_| stream.write_all(b"\r\n"))
                    .is_err()
                {
                    break;
                }
            }
            let _ = stream.write_all(b"0\r\n\r\n");
        });

        let mut config = Config::default();
        config.server.url = Some(format!("http://{address}"));
        config.server.allow_insecure_http = true;
        config.server.timeout_seconds = 5;
        config.agent.state_dir = root.path().join("state");
        config.updates.enabled = true;
        config.updates.public_key = Some(STANDARD.encode(signing.verifying_key().to_bytes()));
        config.updates.install_path = root.path().join("invenqor-agent");
        config.validate().unwrap();

        let error = check_and_stage(&config, "oversized-manifest-agent")
            .await
            .unwrap_err();
        server.join().unwrap();
        assert!(
            format!("{error:#}").contains("65536-byte limit"),
            "unexpected error: {error:#}"
        );
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
    fn only_windows_applies_a_staged_update_inside_the_agent_process() {
        let called = std::cell::Cell::new(false);
        let result = apply_staged_in_process(|| {
            called.set(true);
            Ok(Some("9.0.0".to_string()))
        })
        .unwrap();

        if cfg!(windows) {
            assert!(called.get());
            assert_eq!(result.as_deref(), Some("9.0.0"));
        } else {
            assert!(!called.get());
            assert!(result.is_none());
        }
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

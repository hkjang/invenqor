use crate::collectors::{collect_all, configured, Collector};
use crate::config::Config;
use crate::health::StatusReport;
use crate::identity::HostIdentity;
#[cfg(any(windows, test))]
use crate::model::AssetRecord;
use crate::model::{unix_time, Envelope, EnvelopeKind, Snapshot};
use crate::storage::StateStore;
use crate::transport::{failure_of, Transport};
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

pub struct Agent {
    config: Config,
    identity: HostIdentity,
    collectors: Vec<Arc<dyn Collector>>,
    store: StateStore,
    transport: Option<Transport>,
    status: StatusReport,
}

impl Agent {
    pub fn new(config: Config, identity: HostIdentity, config_path: &Path) -> Result<Self> {
        let collectors = configured(&config.collectors);
        let store = StateStore::open(&config.agent.state_dir, config.agent.max_queue_bytes)?;
        let mut transport = Transport::new(&config.server)?;
        if let (Some(transport), Some(server_url)) =
            (transport.as_mut(), config.server.url.as_deref())
        {
            if transport.bearer_token().is_none() {
                transport.set_bearer_token(store.device_token(server_url));
            }
        }
        let mut status = StatusReport::new(
            identity.agent_id.clone(),
            host_name(),
            config_path.display().to_string(),
            config.agent.state_dir.display().to_string(),
            config.server.url.clone(),
            auth_mode(&config),
            unix_time(),
        );
        if transport
            .as_ref()
            .is_some_and(|value| value.bearer_token().is_some())
        {
            status.record_enrolled(unix_time());
        }
        // Recorded here rather than only in run(): a one-shot invocation writes
        // the same status file, and reporting updates as disabled there would be
        // wrong.
        status.record_update_settings(config.updates.enabled, &config.updates.channel);
        let agent = Self {
            config,
            identity,
            collectors,
            store,
            transport,
            status,
        };
        agent.announce();
        agent.persist_status();
        Ok(agent)
    }

    /// States the effective transport configuration once at start-up. Silence
    /// here was the reason an Agent with a commented out `server.url` looked
    /// identical to a healthy one.
    fn announce(&self) {
        match self.config.server.url.as_deref() {
            Some(url) => info!(
                server_url = url,
                auth_mode = %self.status.auth_mode,
                state_dir = %self.config.agent.state_dir.display(),
                config = %self.status.config_path,
                enrollment = %self.status.enrollment.state,
                interval_seconds = self.config.agent.interval_seconds,
                "agent transport configured"
            ),
            None => warn!(
                config = %self.status.config_path,
                state_dir = %self.config.agent.state_dir.display(),
                status_file = %self.store.status_path().display(),
                "server.url is not configured: inventory is collected into the local queue only, \
                 no registration is attempted, and nothing is sent to a Server"
            ),
        }
    }

    fn persist_status(&self) {
        if let Err(error) = self.store.write_status(&self.status) {
            warn!(error = %format!("{error:#}"), "could not write the Agent status report");
        }
    }

    fn refresh_queue_status(&mut self) {
        let pending = self.store.pending().unwrap_or_default();
        let bytes = self.store.queue_bytes().unwrap_or_default();
        self.status
            .record_queue(pending.len(), bytes, self.config.agent.max_queue_bytes);
    }

    pub fn status(&self) -> &StatusReport {
        &self.status
    }

    pub async fn collect_once(&mut self) -> Result<Snapshot> {
        #[cfg(windows)]
        let loaded_user_sids_before = crate::collectors::loaded_windows_user_sids();
        let collected = collect_all(&self.identity.agent_id, self.collectors.clone()).await;
        #[cfg(windows)]
        let mut snapshot = collected;
        #[cfg(not(windows))]
        let snapshot = collected;
        let previous_hash = self.store.previous_hash();
        let previous_inventory = self.store.previous_inventory()?;
        #[cfg(windows)]
        {
            // A profile can load between package collection and this check. It
            // is authoritative only when it stayed loaded for the whole cycle;
            // otherwise an empty observation could be mistaken for uninstalling
            // every per-user package on that profile.
            let loaded_after = crate::collectors::loaded_windows_user_sids();
            let stable_loaded_sids = loaded_user_sids_before
                .intersection(&loaded_after)
                .cloned()
                .collect();
            retain_unloaded_windows_user_packages(
                &mut snapshot.records,
                &previous_inventory,
                &stable_loaded_sids,
            );
        }
        // A failed collector must not turn all of its assets into false removals.
        let allow_removals = snapshot.errors.is_empty();
        let changes = StateStore::diff(&previous_inventory, &snapshot.records, allow_removals);
        let effective_inventory =
            StateStore::effective_inventory(&previous_inventory, &snapshot.records, allow_removals);
        let mut hash_snapshot = snapshot.clone();
        hash_snapshot.records = effective_inventory.clone();
        let hash = StateStore::snapshot_hash(&hash_snapshot)?;
        let first_inventory = previous_hash.is_none();
        let changed = first_inventory || !changes.is_empty();
        let now = unix_time();
        let heartbeat_due =
            now.saturating_sub(self.store.last_heartbeat()) >= self.config.agent.heartbeat_seconds;

        if changed || heartbeat_due {
            let envelope = Envelope {
                schema_version: 1,
                event_id: Uuid::new_v4().to_string(),
                agent_id: self.identity.agent_id.clone(),
                created_at: now,
                kind: if changed {
                    EnvelopeKind::Inventory
                } else {
                    EnvelopeKind::Heartbeat
                },
                snapshot_hash: hash.clone(),
                snapshot: (changed && first_inventory).then(|| snapshot.clone()),
                changes: if changed && !first_inventory {
                    changes
                } else {
                    Vec::new()
                },
                collection_errors: snapshot.errors.clone(),
            };
            self.store.enqueue(&envelope)?;
            if changed {
                self.store.set_previous_hash(&hash)?;
                self.store.set_previous_inventory(&effective_inventory)?;
            }
            self.store.set_last_heartbeat(now)?;
            info!(
                event_id = %envelope.event_id,
                changed,
                records = snapshot.records.len(),
                errors = snapshot.errors.len(),
                "queued collection event"
            );
        } else {
            info!("inventory unchanged and heartbeat not due");
        }

        self.status
            .record_collection(snapshot.records.len(), snapshot.errors.len(), now);
        self.refresh_queue_status();
        self.persist_status();
        Ok(snapshot)
    }

    pub async fn drain_queue(&mut self) -> Result<usize> {
        let outcome = self.deliver().await;
        self.refresh_queue_status();
        self.persist_status();
        outcome
    }

    async fn deliver(&mut self) -> Result<usize> {
        self.ensure_enrolled().await?;
        if self.transport.is_none() {
            let pending = self.store.pending().unwrap_or_default().len();
            if pending > 0 {
                warn!(
                    pending_events = pending,
                    queue_bytes = self.store.queue_bytes().unwrap_or_default(),
                    limit_bytes = self.config.agent.max_queue_bytes,
                    "no Server is configured, so queued events cannot be delivered; \
                     set server.url in the Agent configuration"
                );
            }
            return Ok(0);
        }
        let mut sent = 0;
        let mut last_request_id = None;
        let mut last_policy_version = None;
        for path in self.store.pending()? {
            let envelope = self.store.read_envelope(&path)?;
            let first_attempt = self
                .transport
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("server transport is unavailable"))?
                .send(&envelope)
                .await;
            let acknowledgement = match first_attempt {
                Ok(value) => value,
                Err(error) => {
                    // A rejected device credential is recoverable exactly once
                    // per cycle: discard it and let registration issue a new one.
                    let stale_credential = crate::transport::is_unauthorized(&error)
                        && self.config.server.bearer_token.is_none();
                    if !stale_credential {
                        log_failure("inventory delivery", &error);
                        self.status.record_delivery_failure(&error, unix_time());
                        return Err(error);
                    }
                    let server_url = self
                        .config
                        .server
                        .url
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("server.url is required"))?;
                    warn!(
                        code = "AGENT_UNAUTHORIZED",
                        request_id = failure_of(&error)
                            .and_then(|value| value.request_id.as_deref())
                            .unwrap_or("-"),
                        "the stored device credential was rejected; discarding it and re-registering"
                    );
                    self.store.clear_device_token(server_url)?;
                    if let Some(transport) = self.transport.as_mut() {
                        transport.set_bearer_token(None);
                    }
                    self.status.enrollment.state = "pending".into();
                    self.ensure_enrolled().await?;
                    let retried = self
                        .transport
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("server transport is unavailable"))?
                        .send(&envelope)
                        .await;
                    match retried {
                        Ok(value) => value,
                        Err(error) => {
                            log_failure("inventory delivery", &error);
                            self.status.record_delivery_failure(&error, unix_time());
                            return Err(error);
                        }
                    }
                }
            };
            if let Some(version) = &acknowledgement.policy_version {
                if !version.is_empty() {
                    last_policy_version = Some(version.clone());
                }
            }
            if acknowledgement.request_id.is_some() {
                last_request_id = acknowledgement.request_id.clone();
            }
            if acknowledgement.spooled {
                info!(
                    event_id = %envelope.event_id,
                    "the Server spooled the event because its database is degraded"
                );
            }
            self.store.acknowledge(&path)?;
            sent += 1;
        }
        if sent > 0 {
            self.status.record_delivery_success(
                sent as u64,
                last_request_id,
                last_policy_version,
                unix_time(),
            );
        }
        Ok(sent)
    }

    async fn ensure_enrolled(&mut self) -> Result<()> {
        let Some(transport) = self.transport.as_mut() else {
            return Ok(());
        };
        if transport.bearer_token().is_some() {
            return Ok(());
        }
        if self.config.server.bearer_token.is_some() {
            return Ok(());
        }
        let server_url = self
            .config
            .server
            .url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("server.url is required"))?;
        let claim = self.store.enrollment_claim(server_url)?;
        let hostname = host_name();
        let now = unix_time();
        self.status.record_enrollment_attempt(now);
        info!(
            agent_id = %self.identity.agent_id,
            hostname = %hostname,
            server_url = server_url,
            fleet_token = transport.has_enrollment_token(),
            attempt = self.status.enrollment.attempts,
            "requesting automatic registration"
        );
        let token = match transport
            .enroll(&self.identity.agent_id, &hostname, &claim)
            .await
        {
            Ok(token) => token,
            Err(error) => {
                log_failure("automatic enrollment", &error);
                self.status.record_enrollment_failure(&error, unix_time());
                self.persist_status();
                return Err(error);
            }
        };
        self.store.set_device_token(server_url, &token)?;
        transport.set_bearer_token(Some(token));
        self.status.record_enrolled(unix_time());
        info!(
            agent_id = %self.identity.agent_id,
            hostname = %hostname,
            "registered automatically and stored the device credential"
        );
        self.persist_status();
        Ok(())
    }

    fn queue_heartbeat(&mut self) -> Result<()> {
        let now = unix_time();
        let envelope = Envelope {
            schema_version: 1,
            event_id: Uuid::new_v4().to_string(),
            agent_id: self.identity.agent_id.clone(),
            created_at: now,
            kind: EnvelopeKind::Heartbeat,
            snapshot_hash: self.store.previous_hash().unwrap_or_default(),
            snapshot: None,
            changes: Vec::new(),
            collection_errors: Vec::new(),
        };
        self.store.enqueue(&envelope)?;
        self.store.set_last_heartbeat(now)?;
        info!(event_id = %envelope.event_id, "queued heartbeat");
        Ok(())
    }

    pub async fn run(mut self) -> Result<()> {
        self.status
            .record_update_settings(self.config.updates.enabled, &self.config.updates.channel);
        self.persist_status();
        if self.config.updates.enabled {
            tokio::spawn(crate::updater::run_checker(
                self.config.clone(),
                self.identity.agent_id.clone(),
            ));
        } else {
            info!(
                "automatic updates are disabled; set updates.enabled and updates.public_key \
                 to let this Agent take signed releases"
            );
        }
        if let Err(error) = self.collect_once().await {
            warn!(error = %format!("{error:#}"), "initial collection cycle failed");
        }
        let collection_interval = Duration::from_secs(self.config.agent.interval_seconds);
        let heartbeat_interval = self.config.agent.heartbeat_seconds;
        let mut next_collection = tokio::time::Instant::now() + collection_interval;
        let mut backoff = 1u64;
        let mut retry_at = None;

        match self.drain_queue().await {
            Ok(sent) => {
                if sent > 0 {
                    info!(sent, "delivered queued events");
                }
            }
            Err(_) => {
                self.report_retry(backoff);
                retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(backoff));
                backoff = backoff
                    .saturating_mul(2)
                    .min(self.config.agent.max_backoff_seconds.max(1));
            }
        }

        loop {
            let heartbeat_elapsed = unix_time().saturating_sub(self.store.last_heartbeat());
            let heartbeat_delay =
                Duration::from_secs(heartbeat_interval.saturating_sub(heartbeat_elapsed));
            let heartbeat_at = tokio::time::Instant::now() + heartbeat_delay;
            let wake_at = retry_at
                .map(|retry| retry.min(next_collection).min(heartbeat_at))
                .unwrap_or_else(|| next_collection.min(heartbeat_at));
            if wait_or_shutdown(wake_at.saturating_duration_since(tokio::time::Instant::now()))
                .await
            {
                return Ok(());
            }

            let now = tokio::time::Instant::now();
            let mut event_queued = false;
            if now >= next_collection {
                match self.collect_once().await {
                    Ok(_) => event_queued = true,
                    Err(error) => {
                        warn!(error = %format!("{error:#}"), "collection cycle failed")
                    }
                }
                next_collection = tokio::time::Instant::now() + collection_interval;
            } else if now >= heartbeat_at {
                match self.queue_heartbeat() {
                    Ok(()) => event_queued = true,
                    Err(error) => {
                        warn!(error = %format!("{error:#}"), "heartbeat queueing failed")
                    }
                }
            }

            let retry_due = retry_at.is_some_and(|retry| now >= retry);
            if !event_queued && !retry_due {
                continue;
            }

            match self.drain_queue().await {
                Ok(sent) => {
                    if sent > 0 {
                        info!(sent, "delivered queued events");
                    }
                    backoff = 1;
                    retry_at = None;
                }
                Err(_) => {
                    self.report_retry(backoff);
                    retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(backoff));
                    backoff = backoff
                        .saturating_mul(2)
                        .min(self.config.agent.max_backoff_seconds.max(1));
                }
            }
        }
    }

    /// The individual failure was already logged with its code; this line adds
    /// the operational consequence, which is what an operator watching a rollout
    /// needs to see.
    fn report_retry(&self, backoff: u64) {
        warn!(
            backoff_seconds = backoff,
            pending_events = self.status.queue.pending_events,
            queue_bytes = self.status.queue.bytes,
            enrollment = %self.status.enrollment.state,
            status_file = %self.store.status_path().display(),
            summary = %self.status.headline(),
            "inventory is not reaching the Server; retrying after the backoff"
        );
    }
}

/// HKEY_USERS contains only profiles whose hives are currently loaded. A user
/// logging off is not an uninstall event, so preserve that SID's last package
/// records until its hive is loaded again and can authoritatively report the
/// current contents.
#[cfg(any(windows, test))]
fn retain_unloaded_windows_user_packages(
    current: &mut Vec<AssetRecord>,
    previous: &[AssetRecord],
    loaded_sids: &std::collections::BTreeSet<String>,
) {
    let mut current_ids: std::collections::BTreeSet<String> = current
        .iter()
        .map(|record| record.asset_id.clone())
        .collect();
    for record in previous {
        if record.category != "software.package" || current_ids.contains(&record.asset_id) {
            continue;
        }
        let manager = record
            .payload
            .get("manager")
            .and_then(|value| value.as_str());
        let scope = record.payload.get("scope").and_then(|value| value.as_str());
        let owner_sid = record
            .payload
            .get("owner_sid")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if manager == Some("windows")
            && scope == Some("user")
            && !owner_sid.is_empty()
            && !loaded_sids.contains(owner_sid)
        {
            current.push(record.clone());
            current_ids.insert(record.asset_id.clone());
        }
    }
    current.sort_by(|left, right| left.asset_id.cmp(&right.asset_id));
}

fn auth_mode(config: &Config) -> &'static str {
    if config.server.url.is_none() {
        return "none";
    }
    if config.server.bearer_token.is_some() {
        return "static_bearer";
    }
    if config.server.client_identity_pem.is_some() {
        return "mtls";
    }
    "device_token"
}

/// Emits one line carrying the stable code, the Server request ID and the
/// remedy, so the Agent journal and the Server console can be joined.
fn log_failure(stage: &str, error: &anyhow::Error) {
    match failure_of(error) {
        Some(failure) => warn!(
            stage,
            code = %failure.code,
            http_status = ?failure.status,
            path = failure.path,
            request_id = failure.request_id.as_deref().unwrap_or("-"),
            server_message = failure.server_message.as_deref().unwrap_or("-"),
            cause = failure.cause.as_deref().unwrap_or("-"),
            remediation = failure.remediation(),
            "Server exchange failed"
        ),
        None => warn!(
            stage,
            code = "AGENT_LOCAL_ERROR",
            error = %format!("{error:#}"),
            "Server exchange failed before a request could be made"
        ),
    }
}

pub fn host_name() -> String {
    crate::platform::hostname()
}

async fn wait_or_shutdown(duration: Duration) -> bool {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            _ = tokio::signal::ctrl_c() => true,
            _ = async {
                match terminate.as_mut() {
                    Some(signal) => { signal.recv().await; }
                    None => std::future::pending::<()>().await,
                }
            } => true,
        }
    }
    #[cfg(windows)]
    {
        // The Service Control Manager does not send a signal: it calls the
        // control handler, which sets a flag. Polling it keeps a stop prompt
        // rather than waiting out a fifteen-minute collection sleep, which the
        // SCM would treat as a hung service and kill.
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            if crate::windows_service::stop_requested() {
                return true;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            tokio::select! {
                _ = tokio::time::sleep(remaining.min(Duration::from_secs(1))) => {}
                _ = tokio::signal::ctrl_c() => return true,
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            _ = tokio::signal::ctrl_c() => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_package(asset_id: &str, sid: &str) -> AssetRecord {
        AssetRecord {
            asset_id: asset_id.into(),
            category: "software.package".into(),
            source: "uninstall registry".into(),
            collected_at: 1,
            payload: json!({
                "manager": "windows",
                "scope": "user",
                "owner_sid": sid,
                "name": "Example"
            }),
        }
    }

    #[test]
    fn unloaded_windows_profile_is_not_mistaken_for_an_uninstall() {
        let sid = "S-1-5-21-100";
        let previous = vec![user_package("package-a", sid)];
        let mut current = Vec::new();
        retain_unloaded_windows_user_packages(&mut current, &previous, &Default::default());
        assert_eq!(current, previous);

        let mut current = Vec::new();
        retain_unloaded_windows_user_packages(
            &mut current,
            &previous,
            &[sid.to_string()].into_iter().collect(),
        );
        assert!(
            current.is_empty(),
            "a loaded hive is authoritative and may report a real uninstall"
        );
    }
}

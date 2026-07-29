use crate::collectors::{collect_all, configured, Collector};
use crate::config::Config;
use crate::identity::HostIdentity;
use crate::model::{unix_time, Envelope, EnvelopeKind, Snapshot};
use crate::storage::StateStore;
use crate::transport::Transport;
use anyhow::Result;
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
}

impl Agent {
    pub fn new(config: Config, identity: HostIdentity) -> Result<Self> {
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
        Ok(Self {
            config,
            identity,
            collectors,
            store,
            transport,
        })
    }

    pub async fn collect_once(&self) -> Result<Snapshot> {
        let snapshot = collect_all(&self.identity.agent_id, self.collectors.clone()).await;
        let previous_hash = self.store.previous_hash();
        let previous_inventory = self.store.previous_inventory()?;
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

        Ok(snapshot)
    }

    pub async fn drain_queue(&mut self) -> Result<usize> {
        self.ensure_enrolled().await?;
        if self.transport.is_none() {
            return Ok(0);
        }
        let mut sent = 0;
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
                Err(error)
                    if crate::transport::is_unauthorized(&error)
                        && self.config.server.bearer_token.is_none() =>
                {
                    let server_url = self
                        .config
                        .server
                        .url
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("server.url is required"))?;
                    self.store.clear_device_token(server_url)?;
                    if let Some(transport) = self.transport.as_mut() {
                        transport.set_bearer_token(None);
                    }
                    self.ensure_enrolled().await?;
                    self.transport
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("server transport is unavailable"))?
                        .send(&envelope)
                        .await?
                }
                Err(error) => return Err(error),
            };
            if let Some(version) = acknowledgement.policy_version {
                info!(policy_version = version, "server policy advertised");
            }
            self.store.acknowledge(&path)?;
            sent += 1;
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
        let server_url = self
            .config
            .server
            .url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("server.url is required"))?;
        let claim = self.store.enrollment_claim(server_url)?;
        let hostname = host_name();
        let token = transport
            .enroll(&self.identity.agent_id, &hostname, &claim)
            .await?;
        self.store.set_device_token(server_url, &token)?;
        transport.set_bearer_token(Some(token));
        info!(
            agent_id = %self.identity.agent_id,
            "agent automatically enrolled and device credential stored"
        );
        Ok(())
    }

    fn queue_heartbeat(&self) -> Result<()> {
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
        if self.config.updates.enabled {
            tokio::spawn(crate::updater::run_checker(
                self.config.clone(),
                self.identity.agent_id.clone(),
            ));
        }
        if let Err(error) = self.collect_once().await {
            warn!(error = %error, "initial collection cycle failed");
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
            Err(error) => {
                warn!(error = %error, backoff_seconds = backoff, "delivery failed");
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
                    Err(error) => warn!(error = %error, "collection cycle failed"),
                }
                next_collection = tokio::time::Instant::now() + collection_interval;
            } else if now >= heartbeat_at {
                match self.queue_heartbeat() {
                    Ok(()) => event_queued = true,
                    Err(error) => warn!(error = %error, "heartbeat queueing failed"),
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
                Err(error) => {
                    warn!(error = %error, backoff_seconds = backoff, "delivery failed");
                    retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(backoff));
                    backoff = backoff
                        .saturating_mul(2)
                        .min(self.config.agent.max_backoff_seconds.max(1));
                }
            }
        }
    }
}

fn host_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
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
    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            _ = tokio::signal::ctrl_c() => true,
        }
    }
}

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

fn default_interval() -> u64 {
    900
}
fn default_heartbeat() -> u64 {
    300
}
fn default_timeout() -> u64 {
    30
}
fn default_state_dir() -> PathBuf {
    PathBuf::from("/var/lib/invenqor-agent")
}
fn default_true() -> bool {
    true
}
fn default_max_processes() -> usize {
    10_000
}
fn default_queue_size() -> u64 {
    100 * 1024 * 1024
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub agent: AgentConfig,
    pub updates: UpdateConfig,
    pub collectors: CollectorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub url: Option<String>,
    pub bearer_token: Option<String>,
    pub ca_file: Option<PathBuf>,
    pub client_identity_pem: Option<PathBuf>,
    pub allow_insecure_http: bool,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub state_dir: PathBuf,
    pub interval_seconds: u64,
    pub heartbeat_seconds: u64,
    pub max_backoff_seconds: u64,
    pub max_queue_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateConfig {
    pub enabled: bool,
    pub channel: String,
    pub check_interval_seconds: u64,
    pub public_key: Option<String>,
    pub install_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CollectorConfig {
    pub os: bool,
    pub cpu: bool,
    pub memory: bool,
    pub disk: bool,
    pub network: bool,
    pub process: bool,
    pub packages: bool,
    pub services: bool,
    pub accounts: bool,
    pub containers: bool,
    pub include_process_cmdline: bool,
    pub max_processes: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            url: None,
            bearer_token: None,
            ca_file: None,
            client_identity_pem: None,
            allow_insecure_http: false,
            timeout_seconds: default_timeout(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            state_dir: default_state_dir(),
            interval_seconds: default_interval(),
            heartbeat_seconds: default_heartbeat(),
            max_backoff_seconds: 3600,
            max_queue_bytes: default_queue_size(),
        }
    }
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            channel: "stable".to_string(),
            check_interval_seconds: 21_600,
            public_key: None,
            install_path: PathBuf::from("/opt/invenqor-agent/bin/invenqor-agent"),
        }
    }
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            os: default_true(),
            cpu: default_true(),
            memory: default_true(),
            disk: default_true(),
            network: default_true(),
            process: default_true(),
            packages: default_true(),
            services: default_true(),
            accounts: default_true(),
            containers: default_true(),
            include_process_cmdline: false,
            max_processes: default_max_processes(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("parse config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.agent.interval_seconds > 0,
            "interval_seconds must be > 0"
        );
        anyhow::ensure!(
            self.agent.heartbeat_seconds > 0,
            "heartbeat_seconds must be > 0"
        );
        anyhow::ensure!(
            self.server.timeout_seconds > 0,
            "timeout_seconds must be > 0"
        );
        anyhow::ensure!(
            self.collectors.max_processes > 0,
            "max_processes must be > 0"
        );
        anyhow::ensure!(
            self.updates.check_interval_seconds >= 300,
            "updates.check_interval_seconds must be at least 300"
        );
        if self.updates.enabled {
            anyhow::ensure!(self.server.url.is_some(), "updates require server.url");
            anyhow::ensure!(
                self.updates
                    .public_key
                    .as_deref()
                    .is_some_and(|v| !v.is_empty()),
                "updates require an Ed25519 public_key"
            );
        }
        if let Some(url) = &self.server.url {
            anyhow::ensure!(
                url.starts_with("https://")
                    || self.server.allow_insecure_http && url.starts_with("http://"),
                "server.url must use HTTPS unless allow_insecure_http is explicitly enabled"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(toml::from_str::<Config>("[agent]\nunknown = true").is_err());
    }
}

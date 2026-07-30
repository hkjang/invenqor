use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use url::{Host, Url};

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
    crate::platform::default_state_dir()
}

fn default_install_path() -> PathBuf {
    crate::platform::default_install_path()
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
    pub enrollment_token: Option<String>,
    pub enrollment_token_file: Option<PathBuf>,
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
            enrollment_token: None,
            enrollment_token_file: None,
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
            install_path: default_install_path(),
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
}

/// What the process can tell about a configuration file before reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigAvailability {
    Readable,
    Missing,
    /// The path exists but this account cannot read it, or cannot traverse a
    /// directory on the way to it.
    Unreadable,
}

impl ConfigAvailability {
    /// `Path::exists()` answers false for a file that is present but denied,
    /// because it treats every stat failure as absence. That turned a
    /// permission problem into "no configuration file was found", and the Agent
    /// then ran on built-in defaults - collecting into the local queue with no
    /// Server for as long as nobody noticed. The two cases need different
    /// answers, so they need different names.
    pub fn inspect(path: &Path) -> Self {
        match std::fs::File::open(path) {
            Ok(_) => Self::Readable,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Missing,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Self::Unreadable,
            // Anything else - a broken symlink target, a directory in the way -
            // is not "absent" either, so it must not silently become defaults.
            Err(_) => Self::Unreadable,
        }
    }

    pub fn is_readable(self) -> bool {
        self == Self::Readable
    }
}

impl Config {
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
            let parsed = Url::parse(url).context("server.url must be a valid URL")?;
            anyhow::ensure!(
                parsed.scheme() == "https"
                    || parsed.scheme() == "http" && self.server.allows_http(),
                "server.url must use HTTPS; private/internal HTTP is accepted automatically, while public HTTP requires allow_insecure_http"
            );
        }
        anyhow::ensure!(
            !(self.server.enrollment_token.is_some()
                && self.server.enrollment_token_file.is_some()),
            "server.enrollment_token and server.enrollment_token_file cannot both be configured"
        );
        if let Some(token) = &self.server.enrollment_token {
            anyhow::ensure!(
                token.trim().len() >= 32,
                "server.enrollment_token must contain at least 32 characters"
            );
        }
        if let Some(path) = &self.server.enrollment_token_file {
            anyhow::ensure!(
                path.is_absolute(),
                "server.enrollment_token_file must be an absolute path"
            );
        }
        Ok(())
    }
}

impl ServerConfig {
    pub fn allows_http(&self) -> bool {
        if self.allow_insecure_http {
            return true;
        }
        let Some(value) = &self.url else {
            return false;
        };
        let Ok(parsed) = Url::parse(value) else {
            return false;
        };
        if parsed.scheme() != "http" {
            return false;
        }
        match parsed.host() {
            Some(Host::Ipv4(address)) => {
                address.is_private() || address.is_loopback() || address.is_link_local()
            }
            Some(Host::Ipv6(address)) => {
                address.is_loopback()
                    || address.is_unique_local()
                    || address.is_unicast_link_local()
            }
            Some(Host::Domain(host)) => {
                host.eq_ignore_ascii_case("localhost")
                    || !host.contains('.')
                    || host.ends_with(".internal")
                    || host.ends_with(".local")
            }
            None => false,
        }
    }

    pub fn resolved_enrollment_token(&self) -> Result<Option<String>> {
        if let Some(token) = &self.enrollment_token {
            return Ok(Some(token.trim().to_string()));
        }
        let Some(path) = &self.enrollment_token_file else {
            return Ok(None);
        };
        let token = std::fs::read_to_string(path)
            .with_context(|| format!("read enrollment token {}", path.display()))?
            .trim()
            .to_string();
        anyhow::ensure!(
            token.len() >= 32,
            "server enrollment token file must contain at least 32 characters"
        );
        Ok(Some(token))
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

    #[test]
    fn url_only_allows_private_http_but_rejects_public_http() {
        let mut config = Config::default();
        config.server.url = Some("http://192.168.10.20:7070".into());
        config.validate().unwrap();
        config.server.url = Some("http://invenqor-server:7070".into());
        config.validate().unwrap();
        config.server.url = Some("http://inventory.example.com:7070".into());
        assert!(config.validate().is_err());
    }
}

use crate::config::Config;
use crate::health::format_unix_utc;
use crate::identity;
use crate::model::unix_time;
use crate::storage::StateStore;
use crate::transport::{failure_of, Transport};
use serde::Serialize;
use std::fmt::Write as _;
use std::net::ToSocketAddrs;
use std::path::Path;
use url::Url;

/// A registration problem is almost always one of six things: no configuration,
/// an unwritable state directory, name resolution, a blocked port, TLS trust, or
/// a Server policy. `--diagnose` walks them in that order and stops guessing:
/// every check reports the concrete value it observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Warn,
    Fail,
    Skip,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Outcome::Pass => "PASS",
            Outcome::Warn => "WARN",
            Outcome::Fail => "FAIL",
            Outcome::Skip => "SKIP",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub outcome: Outcome,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl Check {
    fn new(name: &'static str, outcome: Outcome, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome,
            detail: detail.into(),
            code: None,
            remediation: None,
        }
    }

    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self::new(name, Outcome::Pass, detail)
    }

    fn warn(name: &'static str, detail: impl Into<String>, remediation: impl Into<String>) -> Self {
        let mut check = Self::new(name, Outcome::Warn, detail);
        check.remediation = Some(remediation.into());
        check
    }

    fn fail(name: &'static str, detail: impl Into<String>, remediation: impl Into<String>) -> Self {
        let mut check = Self::new(name, Outcome::Fail, detail);
        check.remediation = Some(remediation.into());
        check
    }

    fn skip(name: &'static str, detail: impl Into<String>) -> Self {
        Self::new(name, Outcome::Skip, detail)
    }

    fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub agent_version: String,
    pub generated_at: u64,
    pub generated_at_utc: String,
    pub agent_id: Option<String>,
    pub hostname: String,
    pub config_path: String,
    pub server_url: Option<String>,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn failed(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.outcome == Outcome::Fail)
    }

    pub fn render(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(
            text,
            "invenqor-agent {} registration diagnosis at {}",
            self.agent_version, self.generated_at_utc
        );
        let _ = writeln!(text, "  host          {}", self.hostname);
        let _ = writeln!(
            text,
            "  agent-id      {}",
            self.agent_id.as_deref().unwrap_or("(not created yet)")
        );
        let _ = writeln!(text, "  config        {}", self.config_path);
        let _ = writeln!(
            text,
            "  server.url    {}",
            self.server_url.as_deref().unwrap_or("(not configured)")
        );
        let _ = writeln!(text);
        let width = self
            .checks
            .iter()
            .map(|check| check.name.len())
            .max()
            .unwrap_or(0);
        for check in &self.checks {
            let _ = writeln!(
                text,
                "  [{}] {:width$}  {}",
                check.outcome.label(),
                check.name,
                check.detail,
                width = width
            );
            if let Some(code) = &check.code {
                let _ = writeln!(
                    text,
                    "         {:width$}  code: {}",
                    "",
                    code,
                    width = width
                );
            }
            if let Some(remediation) = &check.remediation {
                let _ = writeln!(
                    text,
                    "         {:width$}  fix:  {}",
                    "",
                    remediation,
                    width = width
                );
            }
        }
        let _ = writeln!(text);
        let _ = writeln!(
            text,
            "  result: {}",
            if self.failed() {
                "FAILED - the Agent cannot register or deliver inventory yet"
            } else {
                "OK - the Agent can reach the Server and register"
            }
        );
        text
    }
}

/// Runs every check that can be performed without changing state. It never
/// enrolls, never sends an event and never rewrites a credential, so it is safe
/// on a production host at any time.
pub async fn run(config: &Config, config_path: &Path, config_present: bool) -> Report {
    let now = unix_time();
    let mut checks = Vec::new();
    let mut agent_id = None;

    checks.push(if config_present {
        Check::pass(
            "configuration file",
            format!("read {}", config_path.display()),
        )
    } else {
        Check::warn(
            "configuration file",
            format!(
                "{} does not exist; built-in defaults are in use",
                config_path.display()
            ),
            "Install the packaged config.toml and set server.url in it.",
        )
    });

    match identity::load_or_create(&config.agent.state_dir) {
        Ok(identity) => {
            agent_id = Some(identity.agent_id.clone());
            checks.push(Check::pass(
                "state directory",
                format!(
                    "{} is writable, agent-id {}",
                    config.agent.state_dir.display(),
                    identity.agent_id
                ),
            ));
        }
        Err(error) => checks.push(Check::fail(
            "state directory",
            format!("{:#}", error),
            "Create the directory and grant the Agent service account write \
             access with mode 0700.",
        )),
    }

    match StateStore::open(&config.agent.state_dir, config.agent.max_queue_bytes) {
        Ok(store) => {
            let pending = store.pending().unwrap_or_default().len();
            let bytes = store.queue_bytes().unwrap_or_default();
            let credential = config
                .server
                .url
                .as_deref()
                .and_then(|url| store.device_token(url));
            checks.push(Check::new(
                "durable queue",
                if bytes >= config.agent.max_queue_bytes {
                    Outcome::Fail
                } else if pending > 0 {
                    Outcome::Warn
                } else {
                    Outcome::Pass
                },
                format!(
                    "{pending} undelivered event(s), {bytes} of {} bytes used",
                    config.agent.max_queue_bytes
                ),
            ));
            checks.push(match &credential {
                Some(_) => Check::pass(
                    "stored credential",
                    "a device credential exists for this Server URL",
                ),
                None => Check::new(
                    "stored credential",
                    Outcome::Warn,
                    "no device credential is stored yet for this Server URL",
                ),
            });
        }
        Err(error) => checks.push(Check::fail(
            "durable queue",
            format!("{:#}", error),
            "Grant the Agent write access to the state directory.",
        )),
    }

    let Some(server_url) = config.server.url.clone() else {
        checks.push(Check::fail(
            "server.url",
            "no Server URL is configured, so the Agent will never register",
            "Set server.url in the Agent configuration to the Server scheme, \
             host and port, for example https://inventory.example:7070.",
        ));
        return finish(config, config_path, agent_id, now, checks);
    };

    let parsed = match Url::parse(&server_url) {
        Ok(value) => {
            checks.push(Check::pass(
                "server.url",
                format!(
                    "{server_url} (scheme {}, host {}, port {})",
                    value.scheme(),
                    value.host_str().unwrap_or("-"),
                    value
                        .port_or_known_default()
                        .map(|port| port.to_string())
                        .unwrap_or_else(|| "-".into())
                ),
            ));
            if value.scheme() == "http" {
                checks.push(Check::warn(
                    "transport encryption",
                    "the Agent is configured for plain HTTP",
                    "Use HTTPS whenever the traffic leaves a trusted network.",
                ));
            } else {
                checks.push(Check::pass("transport encryption", "HTTPS is configured"));
            }
            Some(value)
        }
        Err(error) => {
            checks.push(Check::fail(
                "server.url",
                format!("{server_url} is not a valid URL: {error}"),
                "Write the full URL including the scheme, for example \
                 https://inventory.example:7070.",
            ));
            None
        }
    };

    if let Some(parsed) = &parsed {
        checks.push(resolve_check(parsed));
    }

    let transport = match Transport::new(&config.server) {
        Ok(Some(transport)) => Some(transport),
        Ok(None) => None,
        Err(error) => {
            checks.push(Check::fail(
                "transport client",
                format!("{:#}", error),
                "Verify server.ca_file and server.client_identity_pem are \
                 readable PEM files.",
            ));
            None
        }
    };

    let Some(mut transport) = transport else {
        return finish(config, config_path, agent_id, now, checks);
    };
    // Diagnosis must describe the credential the running service would use.
    if transport.bearer_token().is_none() {
        if let Ok(store) = StateStore::open(&config.agent.state_dir, config.agent.max_queue_bytes) {
            transport.set_bearer_token(store.device_token(&server_url));
        }
    }

    match transport.health().await {
        Ok(status) => checks.push(Check::new(
            "server reachability",
            if status == "READY" {
                Outcome::Pass
            } else {
                Outcome::Warn
            },
            format!("GET /health/ready answered {status}"),
        )),
        Err(error) => checks.push(failure_check("server reachability", &error)),
    }

    match transport.preflight().await {
        Ok(preflight) => {
            checks.push(Check::pass(
                "server identity",
                format!(
                    "Invenqor Server {} (pod {}, database {})",
                    preflight.server_version, preflight.instance_id, preflight.database_mode
                ),
            ));
            checks.push(Check::pass(
                "observed source address",
                format!(
                    "the Server sees this host as {}",
                    preflight.observed_source_ip
                ),
            ));
            let enrollment = &preflight.enrollment;
            checks.push(if enrollment.would_enroll {
                Check::pass(
                    "registration policy",
                    format!(
                        "mode {}, network {}: this host may register",
                        enrollment.mode, enrollment.network_mode
                    ),
                )
                .with_code(enrollment.reason.clone())
            } else {
                Check::fail(
                    "registration policy",
                    format!(
                        "mode {}, network {}: {}",
                        enrollment.mode, enrollment.network_mode, enrollment.detail
                    ),
                    policy_remediation(&enrollment.reason, enrollment.token_required),
                )
                .with_code(enrollment.reason.clone())
            });
            checks.push(match preflight.credential.state.as_str() {
                "valid" => Check::pass(
                    "device credential",
                    format!(
                        "accepted by the Server as agent {} ({})",
                        preflight.credential.agent_id.as_deref().unwrap_or("-"),
                        preflight.credential.auth_method.as_deref().unwrap_or("-")
                    ),
                ),
                "absent" => Check::skip(
                    "device credential",
                    "no credential was presented; the next cycle registers one",
                ),
                "invalid" => Check::warn(
                    "device credential",
                    "the Server rejected the stored credential",
                    "Delete device-credential.json in the state directory; the \
                     Agent registers again automatically.",
                ),
                "blocked" => Check::fail(
                    "device credential",
                    "this Agent is blocked on the Server",
                    "Unblock the Agent on the console Agent page.",
                ),
                other => Check::warn(
                    "device credential",
                    format!("the Server reported credential state {other}"),
                    "Check Server health; it could not verify the credential.",
                ),
            });
        }
        Err(error) => {
            let check = failure_check("registration preflight", &error);
            // A Server older than 0.2.6 answers 404 through the console router
            // with NOT_FOUND. A current Server answering AGENT_ENDPOINT_NOT_FOUND
            // means the URL is wrong, which is a real fault, not a missing feature.
            let unsupported = failure_of(&error).is_some_and(|failure| {
                failure.status == Some(404)
                    && failure
                        .server_code
                        .as_deref()
                        .is_none_or(|code| code == "NOT_FOUND")
            });
            checks.push(if unsupported {
                Check::warn(
                    "registration preflight",
                    "this Server does not implement /v1/agent/preflight",
                    "Upgrade the Server to 0.2.6 or later to diagnose \
                     registration policy from the Agent.",
                )
            } else {
                check
            });
        }
    }

    // Automatic updates are the other thing an operator cannot see from outside.
    checks.push(update_check(config));

    finish(config, config_path, agent_id, now, checks)
}

/// Reports whether this host can actually take a signed release. The three ways
/// it silently cannot are: updates switched off, no pinned public key, and a
/// staged update that nothing privileged ever applies.
fn update_check(config: &Config) -> Check {
    if !config.updates.enabled {
        return Check::skip(
            "automatic updates",
            "disabled in the configuration; releases must be installed by hand",
        );
    }
    if config
        .updates
        .public_key
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return Check::fail(
            "automatic updates",
            "enabled without a pinned Ed25519 public key",
            "Set updates.public_key to the release signing key; without it no \
             update can be verified.",
        );
    }
    let pending = config.agent.state_dir.join("updates/pending.json");
    if pending.exists() {
        return Check::warn(
            "automatic updates",
            format!(
                "a verified update is staged at {} and is waiting to be installed",
                pending.display()
            ),
            "systemd applies it through invenqor-agent-update.path; on OpenRC and \
             SysV it installs at the next service restart, or run \
             --apply-pending-update as root now.",
        );
    }
    Check::pass(
        "automatic updates",
        format!(
            "channel {}, checked every {} seconds",
            config.updates.channel, config.updates.check_interval_seconds
        ),
    )
}

fn finish(
    config: &Config,
    config_path: &Path,
    agent_id: Option<String>,
    now: u64,
    checks: Vec<Check>,
) -> Report {
    Report {
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        generated_at: now,
        generated_at_utc: format_unix_utc(now),
        agent_id,
        hostname: crate::scheduler::host_name(),
        config_path: config_path.display().to_string(),
        server_url: config.server.url.clone(),
        checks,
    }
}

fn resolve_check(url: &Url) -> Check {
    let Some(host) = url.host_str() else {
        return Check::fail(
            "name resolution",
            "server.url has no host",
            "Include the host name or IP address in server.url.",
        );
    };
    let port = url.port_or_known_default().unwrap_or(7070);
    match (host, port).to_socket_addrs() {
        Ok(addresses) => {
            let list: Vec<String> = addresses.map(|value| value.to_string()).collect();
            if list.is_empty() {
                Check::fail(
                    "name resolution",
                    format!("{host} resolved to no address"),
                    "Publish an A or AAAA record for the Server, or use its IP \
                     address in server.url.",
                )
            } else {
                Check::pass(
                    "name resolution",
                    format!("{host} resolves to {}", list.join(", ")),
                )
            }
        }
        Err(error) => Check::fail(
            "name resolution",
            format!("{host}:{port} could not be resolved: {error}"),
            "Fix DNS on this host, add the Server to /etc/hosts, or use its IP \
             address in server.url.",
        ),
    }
}

fn failure_check(name: &'static str, error: &anyhow::Error) -> Check {
    match failure_of(error) {
        Some(failure) => {
            let mut detail = failure
                .cause
                .clone()
                .or_else(|| failure.server_message.clone())
                .unwrap_or_else(|| failure.code.clone());
            if let Some(status) = failure.status {
                detail = format!("HTTP {status}: {detail}");
            }
            if let Some(request_id) = &failure.request_id {
                detail = format!("{detail} (server request_id {request_id})");
            }
            Check::fail(name, detail, failure.remediation()).with_code(failure.code.clone())
        }
        None => Check::fail(
            name,
            format!("{error:#}"),
            "Inspect the Agent journal for the preceding error.",
        ),
    }
}

fn policy_remediation(reason: &str, token_required: bool) -> String {
    match reason {
        "AGENT_AUTO_ENROLLMENT_DISABLED" => {
            "Enable automatic registration in Settings > Agent registration, or \
             provision this device manually and set server.bearer_token."
                .into()
        }
        "AGENT_SOURCE_NOT_ALLOWED" => {
            "Add the observed source address, or its CIDR, to the registration \
             allowlist in Settings > Agent registration."
                .into()
        }
        "AGENT_ENROLLMENT_UNAUTHORIZED" if token_required => {
            "The Server requires a fleet registration token. Issue one in the \
             console and write it to server.enrollment_token_file."
                .into()
        }
        _ => "Open Server 진단 로그 in the console and search for this code.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("invenqor-diagnose-{}", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn missing_server_url_fails_the_report_before_any_network_call() {
        let mut config = Config::default();
        config.agent.state_dir = temp_dir();
        let report = run(&config, Path::new("/etc/invenqor-agent/config.toml"), false).await;
        assert!(report.failed());
        let rendered = report.render();
        assert!(rendered.contains("[FAIL] server.url"));
        assert!(rendered.contains("no Server URL is configured"));
        assert!(rendered.contains("FAILED"));
        // A missing configuration file must be reported, not silently defaulted.
        assert!(rendered.contains("[WARN] configuration file"));
        let _ = std::fs::remove_dir_all(config.agent.state_dir);
    }

    #[tokio::test]
    async fn unreachable_server_is_reported_with_its_cause_and_remedy() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut config = Config::default();
        config.agent.state_dir = temp_dir();
        config.server.url = Some(format!("http://{address}"));
        config.server.allow_insecure_http = true;
        config.server.timeout_seconds = 2;
        let report = run(&config, Path::new("/etc/invenqor-agent/config.toml"), true).await;
        assert!(report.failed());
        let rendered = report.render();
        assert!(rendered.contains("[PASS] name resolution"));
        assert!(rendered.contains("[FAIL] server reachability"));
        assert!(rendered.contains("SERVER_UNREACHABLE"));
        assert!(rendered.to_ascii_lowercase().contains("refused"));
        let _ = std::fs::remove_dir_all(config.agent.state_dir);
    }

    #[test]
    fn report_renders_every_outcome_label() {
        let report = Report {
            agent_version: "0.0.0".into(),
            generated_at: 0,
            generated_at_utc: format_unix_utc(0),
            agent_id: None,
            hostname: "host".into(),
            config_path: "/etc/invenqor-agent/config.toml".into(),
            server_url: None,
            checks: vec![
                Check::pass("a", "ok"),
                Check::warn("b", "careful", "do this"),
                Check::skip("c", "not applicable"),
            ],
        };
        let rendered = report.render();
        assert!(rendered.contains("[PASS] a"));
        assert!(rendered.contains("[WARN] b"));
        assert!(rendered.contains("[SKIP] c"));
        assert!(!report.failed());
        assert!(rendered.contains("OK - the Agent can reach the Server"));
    }
}

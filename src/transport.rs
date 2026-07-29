use crate::config::ServerConfig;
use crate::model::Envelope;
use anyhow::{Context, Result};
use reqwest::{Certificate, Client, Identity};
use serde::{Deserialize, Serialize};
use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

/// Every failed exchange with the Server carries a stable code so the Agent
/// log, `--diagnose` and `status.json` all name the same problem, and so an
/// operator can match it against the Server diagnostics console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeFailure {
    pub code: String,
    pub operation: &'static str,
    pub path: &'static str,
    pub status: Option<u16>,
    pub server_code: Option<String>,
    pub server_message: Option<String>,
    pub request_id: Option<String>,
    pub cause: Option<String>,
}

impl ExchangeFailure {
    /// A one-line remedy the operator can act on without reading the source.
    pub fn remediation(&self) -> &'static str {
        match self.code.as_str() {
            "SERVER_UNREACHABLE" => {
                "The Server did not accept a TCP connection. Verify server.url, \
                 DNS, routing and that TCP 7070 is open outbound."
            }
            "SERVER_TIMEOUT" => {
                "The Server accepted the connection but did not answer in time. \
                 Raise server.timeout_seconds or inspect the proxy in between."
            }
            "TLS_REJECTED" => {
                "TLS verification failed. Point server.ca_file at the private CA \
                 certificate that signed the Server certificate."
            }
            "SERVER_RESPONSE_NOT_JSON" => {
                "A proxy or the console answered instead of the Agent API. Set \
                 server.url to the scheme, host and port only."
            }
            "AGENT_ENDPOINT_NOT_FOUND" => {
                "server.url carries a path or points at another product. Use the \
                 scheme, host and port only."
            }
            "AGENT_AUTO_ENROLLMENT_DISABLED" => {
                "Automatic registration is disabled on the Server. Enable it in \
                 Settings > Agent registration or provision this device manually."
            }
            "AGENT_SOURCE_NOT_ALLOWED" => {
                "The Server registration allowlist rejects this host's source IP. \
                 Add the address or its CIDR in the console."
            }
            "AGENT_ENROLLMENT_UNAUTHORIZED" => {
                "The Server requires a fleet registration token. Write the issued \
                 token to server.enrollment_token_file."
            }
            "AGENT_ENROLLMENT_RATE_LIMITED" | "AGENT_RATE_LIMITED" => {
                "The Server is rate limiting this source. Confirm a single Agent \
                 process per host and let the retry backoff settle."
            }
            "AGENT_ALREADY_CLAIMED" => {
                "This agent-id is already claimed by another device, usually a \
                 cloned disk image. Delete agent-id and enrollment-claim.json."
            }
            "AGENT_BLOCKED" => "An administrator blocked this Agent. Unblock it on the Agent page.",
            "AGENT_UNAUTHORIZED" => {
                "The stored device credential is no longer valid. It is removed \
                 automatically so the next cycle re-registers."
            }
            "EVENT_TOO_LARGE" | "INVALID_EVENT" => {
                "The Server rejected the event payload. Upgrade the Agent so its \
                 schema matches the Server."
            }
            "AGENT_ENROLLMENT_POLICY_UNAVAILABLE" | "AGENT_AUTH_UNAVAILABLE" => {
                "The Server cannot reach its metadata database. Check Server \
                 health and the PostgreSQL settings."
            }
            _ => {
                "Compare this code with Server diagnostics (Server 진단 로그) using \
                 the reported request_id."
            }
        }
    }
}

impl fmt::Display for ExchangeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} failed code={}", self.operation, self.code)?;
        if let Some(status) = self.status {
            write!(formatter, " http_status={status}")?;
        }
        write!(formatter, " path={}", self.path)?;
        if let Some(request_id) = &self.request_id {
            if !request_id.is_empty() {
                write!(formatter, " request_id={request_id}")?;
            }
        }
        if let Some(cause) = &self.cause {
            write!(formatter, " cause={cause}")?;
        }
        if let Some(message) = &self.server_message {
            if !message.is_empty() {
                write!(formatter, " server_message={message}")?;
            }
        }
        write!(formatter, " remediation={}", self.remediation())
    }
}

impl StdError for ExchangeFailure {}

pub fn failure_of(error: &anyhow::Error) -> Option<&ExchangeFailure> {
    error.downcast_ref::<ExchangeFailure>()
}

pub fn is_unauthorized(error: &anyhow::Error) -> bool {
    failure_of(error).is_some_and(|failure| failure.code == "AGENT_UNAUTHORIZED")
}

#[derive(Clone)]
pub struct Transport {
    client: Client,
    base_url: String,
    bearer_token: Option<String>,
    enrollment_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServerAcknowledgement {
    pub accepted: bool,
    #[serde(default)]
    pub policy_version: Option<String>,
    #[serde(default)]
    pub duplicate: bool,
    #[serde(default)]
    pub spooled: bool,
    #[serde(skip)]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EnrollmentResponse {
    token: String,
}

/// What the Server reports about this Agent before any state is created.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Preflight {
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub server_version: String,
    #[serde(default)]
    pub instance_id: String,
    #[serde(default)]
    pub database_mode: String,
    #[serde(default)]
    pub observed_source_ip: String,
    pub enrollment: PreflightEnrollment,
    pub credential: PreflightCredential,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PreflightEnrollment {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub token_required: bool,
    #[serde(default)]
    pub token_presented: bool,
    #[serde(default)]
    pub network_mode: String,
    #[serde(default)]
    pub network_allowed: bool,
    #[serde(default)]
    pub would_enroll: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PreflightCredential {
    #[serde(default)]
    pub presented: bool,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub auth_method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct APIErrorEnvelope {
    error: APIErrorBody,
    #[serde(default)]
    request_id: String,
}

#[derive(Debug, Deserialize)]
struct APIErrorBody {
    code: String,
    message: String,
}

impl Transport {
    pub fn new(config: &ServerConfig) -> Result<Option<Self>> {
        let Some(base_url) = config.url.as_ref() else {
            return Ok(None);
        };
        let mut builder = Client::builder()
            .https_only(!config.allows_http())
            .timeout(Duration::from_secs(config.timeout_seconds))
            .user_agent(concat!("invenqor-agent/", env!("CARGO_PKG_VERSION")));

        if let Some(path) = &config.ca_file {
            let pem = std::fs::read(path)
                .with_context(|| format!("read private CA certificate {}", path.display()))?;
            builder = builder.add_root_certificate(
                Certificate::from_pem(&pem)
                    .with_context(|| format!("parse CA certificate {}", path.display()))?,
            );
        }
        if let Some(path) = &config.client_identity_pem {
            let pem = std::fs::read(path)
                .with_context(|| format!("read client identity {}", path.display()))?;
            builder = builder.identity(
                Identity::from_pem(&pem)
                    .with_context(|| format!("parse client identity {}", path.display()))?,
            );
        }

        Ok(Some(Self {
            client: builder.build().context("build HTTPS client")?,
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer_token: config.bearer_token.clone(),
            enrollment_token: config.resolved_enrollment_token()?,
        }))
    }

    pub fn set_bearer_token(&mut self, token: Option<String>) {
        self.bearer_token = token;
    }

    pub fn bearer_token(&self) -> Option<&str> {
        self.bearer_token.as_deref()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn has_enrollment_token(&self) -> bool {
        self.enrollment_token.is_some()
    }

    /// Reports what the Server would do with this Agent without creating,
    /// consuming or invalidating anything, which is what makes it safe to call
    /// from `--diagnose` on a production host.
    pub async fn preflight(&self) -> Result<Preflight> {
        const OPERATION: &str = "registration preflight";
        const PATH: &str = "/v1/agent/preflight";
        let mut request = self
            .client
            .get(format!("{}{PATH}", self.base_url))
            .header("Accept", "application/json");
        if let Some(token) = &self.enrollment_token {
            request = request.header("X-Invenqor-Enrollment-Token", token);
        }
        if let Some(token) = &self.bearer_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| transport_failure(error, OPERATION, PATH))?;
        if !response.status().is_success() {
            return Err(server_rejection(response, OPERATION, PATH).await.into());
        }
        decode_json::<Preflight>(response, OPERATION, PATH).await
    }

    /// Confirms the Agent is talking to an Invenqor Server at all. Reachability
    /// has to be separable from authorization, otherwise a firewall and a
    /// registration policy look identical in the log.
    pub async fn health(&self) -> Result<String> {
        const OPERATION: &str = "server health check";
        const PATH: &str = "/health/ready";
        let response = self
            .client
            .get(format!("{}{PATH}", self.base_url))
            .send()
            .await
            .map_err(|error| transport_failure(error, OPERATION, PATH))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<serde_json::Value>(&body).ok();
        let Some(parsed) = parsed else {
            return Err(ExchangeFailure {
                code: "SERVER_RESPONSE_NOT_JSON".into(),
                operation: OPERATION,
                path: PATH,
                status: Some(status.as_u16()),
                server_code: None,
                server_message: Some(first_line(&body)),
                request_id: None,
                cause: None,
            }
            .into());
        };
        Ok(parsed
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("UNKNOWN")
            .to_string())
    }

    pub async fn enroll(
        &self,
        agent_id: &str,
        hostname: &str,
        claim_token: &str,
    ) -> Result<String> {
        const OPERATION: &str = "automatic enrollment";
        const PATH: &str = "/v1/agent/enroll";
        let mut request = self
            .client
            .post(format!("{}{PATH}", self.base_url))
            .header("X-Invenqor-Agent-Id", agent_id)
            .json(&serde_json::json!({
                "agent_id": agent_id,
                "hostname": hostname,
                "claim_token": claim_token,
            }));
        if let Some(token) = &self.enrollment_token {
            request = request.header("X-Invenqor-Enrollment-Token", token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| transport_failure(error, OPERATION, PATH))?;
        if !response.status().is_success() {
            return Err(server_rejection(response, OPERATION, PATH).await.into());
        }
        let enrollment = decode_json::<EnrollmentResponse>(response, OPERATION, PATH).await?;
        anyhow::ensure!(
            enrollment.token.starts_with("ivq_at_"),
            "automatic enrollment returned an invalid device token"
        );
        Ok(enrollment.token)
    }

    pub async fn send(&self, envelope: &Envelope) -> Result<ServerAcknowledgement> {
        const OPERATION: &str = "inventory delivery";
        const PATH: &str = "/v1/agent/events";
        let mut request = self
            .client
            .post(format!("{}{PATH}", self.base_url))
            .header("X-Invenqor-Agent-Id", &envelope.agent_id)
            .header("X-Invenqor-Event-Id", &envelope.event_id)
            .json(envelope);
        if let Some(token) = &self.bearer_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| transport_failure(error, OPERATION, PATH))?;
        if !response.status().is_success() {
            return Err(server_rejection(response, OPERATION, PATH).await.into());
        }
        let request_id = header_value(&response, "x-request-id");
        let mut acknowledgement =
            decode_json::<ServerAcknowledgement>(response, OPERATION, PATH).await?;
        acknowledgement.request_id = request_id;
        anyhow::ensure!(acknowledgement.accepted, "server did not accept event");
        Ok(acknowledgement)
    }
}

fn header_value(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

/// Decodes a successful response, turning "the Server answered with HTML"
/// into its own diagnosis instead of an opaque serde error.
async fn decode_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    operation: &'static str,
    path: &'static str,
) -> Result<T> {
    let status = response.status().as_u16();
    let request_id = header_value(&response, "x-request-id");
    let body = response
        .bytes()
        .await
        .map_err(|error| transport_failure(error, operation, path))?;
    serde_json::from_slice::<T>(&body).map_err(|error| {
        ExchangeFailure {
            code: "SERVER_RESPONSE_NOT_JSON".into(),
            operation,
            path,
            status: Some(status),
            server_code: None,
            server_message: Some(first_line(&String::from_utf8_lossy(
                &body[..body.len().min(256)],
            ))),
            request_id,
            cause: Some(error.to_string()),
        }
        .into()
    })
}

async fn server_rejection(
    response: reqwest::Response,
    operation: &'static str,
    path: &'static str,
) -> ExchangeFailure {
    let status = response.status();
    let request_id_header = header_value(&response, "x-request-id");
    let bytes = response.bytes().await.unwrap_or_default();
    let limited = &bytes[..bytes.len().min(4096)];
    let parsed = serde_json::from_slice::<APIErrorEnvelope>(limited).ok();
    let server_code = parsed.as_ref().map(|value| value.error.code.clone());
    let request_id = parsed
        .as_ref()
        .map(|value| value.request_id.clone())
        .filter(|value| !value.is_empty())
        .or(request_id_header);
    // The Server's own code is the most precise label available; fall back to
    // the status so an intermediary's bare error is still actionable.
    let code = match (&server_code, status.as_u16()) {
        (Some(value), _) if !value.is_empty() => value.clone(),
        (_, 401) => "AGENT_UNAUTHORIZED".to_string(),
        (_, 403) => "AGENT_FORBIDDEN".to_string(),
        (_, 404) => "AGENT_ENDPOINT_NOT_FOUND".to_string(),
        (_, 405) => "AGENT_ENDPOINT_METHOD_NOT_ALLOWED".to_string(),
        (_, 429) => "AGENT_RATE_LIMITED".to_string(),
        (_, 502..=504) => "SERVER_GATEWAY_ERROR".to_string(),
        (_, code) => format!("SERVER_HTTP_{code}"),
    };
    ExchangeFailure {
        code,
        operation,
        path,
        status: Some(status.as_u16()),
        server_code,
        server_message: parsed
            .as_ref()
            .map(|value| value.error.message.clone())
            .or_else(|| Some(first_line(&String::from_utf8_lossy(limited)))),
        request_id,
        cause: None,
    }
}

fn transport_failure(
    error: reqwest::Error,
    operation: &'static str,
    path: &'static str,
) -> anyhow::Error {
    let cause = cause_chain(&error);
    let code = if error.is_timeout() {
        "SERVER_TIMEOUT"
    } else if error.is_connect() {
        if cause.to_ascii_lowercase().contains("certificate")
            || cause.to_ascii_lowercase().contains("tls")
        {
            "TLS_REJECTED"
        } else {
            "SERVER_UNREACHABLE"
        }
    } else if error.is_builder() {
        "AGENT_REQUEST_INVALID"
    } else if error.is_redirect() {
        "SERVER_REDIRECT_LOOP"
    } else {
        "SERVER_TRANSPORT_ERROR"
    };
    ExchangeFailure {
        code: code.into(),
        operation,
        path,
        status: None,
        server_code: None,
        server_message: None,
        request_id: None,
        cause: Some(cause),
    }
    .into()
}

/// reqwest hides the useful part - "Connection refused", "certificate has
/// expired" - behind `source()`. Flatten the chain so one log line is enough.
fn cause_chain(error: &(dyn StdError + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    let mut current = error.source();
    while let Some(source) = current {
        let text = source.to_string();
        if !parts.iter().any(|existing| existing == &text) {
            parts.push(text);
        }
        current = source.source();
    }
    parts.join(": ")
}

fn first_line(value: &str) -> String {
    let line = value.trim().lines().next().unwrap_or_default().trim();
    if line.len() > 200 {
        format!("{}…", &line[..200])
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::model::{unix_time, EnvelopeKind};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            let Some(headers_end) = request.windows(4).position(|v| v == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= headers_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8_lossy(&request).to_string()
    }

    /// Serves one canned response from a detached thread. The handle is
    /// deliberately not joined: joining before the client connects would block
    /// on `accept` forever.
    fn serve_once(status_line: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            write!(
                stream,
                "{status_line}\r\nContent-Type: application/json\r\nX-Request-Id: server-request-42\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        });
        format!("http://{address}")
    }

    fn insecure(url: String) -> ServerConfig {
        ServerConfig {
            url: Some(url),
            allow_insecure_http: true,
            timeout_seconds: 2,
            ..ServerConfig::default()
        }
    }

    #[tokio::test]
    async fn sends_identity_auth_and_decodes_acknowledgement() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let body = br#"{"accepted":true,"policy_version":"test-policy"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Request-Id: server-request-42\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
            request
        });

        let config = ServerConfig {
            bearer_token: Some("device-token".into()),
            ..insecure(format!("http://{address}"))
        };
        let transport = Transport::new(&config).unwrap().unwrap();
        let envelope = Envelope {
            schema_version: 1,
            event_id: "event-1".into(),
            agent_id: "agent-1".into(),
            created_at: unix_time(),
            kind: EnvelopeKind::Heartbeat,
            snapshot_hash: "hash".into(),
            snapshot: None,
            changes: Vec::new(),
            collection_errors: Vec::new(),
        };
        let acknowledgement = transport.send(&envelope).await.unwrap();
        assert_eq!(
            acknowledgement.policy_version.as_deref(),
            Some("test-policy")
        );
        assert_eq!(
            acknowledgement.request_id.as_deref(),
            Some("server-request-42")
        );
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.contains("x-invenqor-agent-id: agent-1"));
        assert!(request.contains("x-invenqor-event-id: event-1"));
        assert!(request.contains("authorization: bearer device-token"));
        assert!(request.contains("\"event_id\":\"event-1\""));
    }

    #[tokio::test]
    async fn enrolls_with_only_the_server_url() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let body = br#"{"token":"ivq_at_device-token"}"#;
            write!(
                stream,
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
            request
        });
        let config = insecure(format!("http://{address}"));
        let transport = Transport::new(&config).unwrap().unwrap();
        let token = transport
            .enroll(
                "00000000-0000-0000-0000-000000000001",
                "url-only-host",
                "ivq_ec_claim",
            )
            .await
            .unwrap();
        assert_eq!(token, "ivq_at_device-token");
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("post /v1/agent/enroll "));
        assert!(!request.contains("x-invenqor-enrollment-token"));
        assert!(request.contains("\"hostname\":\"url-only-host\""));
    }

    #[tokio::test]
    async fn enrollment_error_keeps_the_server_code_and_request_id() {
        let url = serve_once(
            "HTTP/1.1 403 Forbidden",
            r#"{"error":{"code":"AGENT_SOURCE_NOT_ALLOWED","message":"The Agent source IP is not permitted by the enrollment policy."},"request_id":"server-request-42"}"#,
        );
        let config = insecure(url);
        let transport = Transport::new(&config).unwrap().unwrap();
        let error = transport
            .enroll(
                "00000000-0000-0000-0000-000000000001",
                "blocked-host",
                "ivq_ec_claim",
            )
            .await
            .unwrap_err();
        let failure = failure_of(&error).expect("classified failure");
        assert_eq!(failure.code, "AGENT_SOURCE_NOT_ALLOWED");
        assert_eq!(failure.request_id.as_deref(), Some("server-request-42"));
        assert_eq!(failure.status, Some(403));
        assert!(failure.remediation().contains("allowlist"));
        let rendered = error.to_string();
        assert!(rendered.contains("path=/v1/agent/enroll"));
        assert!(rendered.contains("remediation="));
    }

    #[tokio::test]
    async fn unreachable_server_reports_the_operating_system_cause() {
        // Binding and dropping yields a port nothing listens on.
        let address = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let config = insecure(format!("http://{address}"));
        let transport = Transport::new(&config).unwrap().unwrap();
        let error = transport
            .enroll(
                "00000000-0000-0000-0000-000000000001",
                "offline-host",
                "ivq_ec_claim",
            )
            .await
            .unwrap_err();
        let failure = failure_of(&error).expect("classified failure");
        assert_eq!(failure.code, "SERVER_UNREACHABLE");
        let cause = failure.cause.clone().unwrap_or_default();
        assert!(
            cause.to_ascii_lowercase().contains("refused"),
            "expected the operating system cause, got {cause}"
        );
    }

    #[tokio::test]
    async fn console_html_is_reported_as_a_wrong_url() {
        let url = serve_once("HTTP/1.1 200 OK", "<!doctype html><title>Console</title>");
        let config = insecure(url);
        let transport = Transport::new(&config).unwrap().unwrap();
        let error = transport
            .enroll(
                "00000000-0000-0000-0000-000000000001",
                "proxied-host",
                "ivq_ec_claim",
            )
            .await
            .unwrap_err();
        let failure = failure_of(&error).expect("classified failure");
        assert_eq!(failure.code, "SERVER_RESPONSE_NOT_JSON");
        assert_eq!(failure.request_id.as_deref(), Some("server-request-42"));
        assert!(failure.remediation().contains("scheme, host and port"));
    }

    #[tokio::test]
    async fn unauthorized_delivery_is_recognised_for_re_enrollment() {
        let url = serve_once(
            "HTTP/1.1 401 Unauthorized",
            r#"{"error":{"code":"AGENT_UNAUTHORIZED","message":"The agent credential is invalid."},"request_id":"server-request-42"}"#,
        );
        let config = ServerConfig {
            bearer_token: Some("stale".into()),
            ..insecure(url)
        };
        let transport = Transport::new(&config).unwrap().unwrap();
        let envelope = Envelope {
            schema_version: 1,
            event_id: "event-1".into(),
            agent_id: "agent-1".into(),
            created_at: unix_time(),
            kind: EnvelopeKind::Heartbeat,
            snapshot_hash: "hash".into(),
            snapshot: None,
            changes: Vec::new(),
            collection_errors: Vec::new(),
        };
        let error = transport.send(&envelope).await.unwrap_err();
        assert!(is_unauthorized(&error));
    }

    #[tokio::test]
    async fn preflight_reports_the_server_decision() {
        let url = serve_once(
            "HTTP/1.1 200 OK",
            r#"{"request_id":"r-1","server_version":"0.2.6","instance_id":"pod-a","database_mode":"POSTGRES","observed_source_ip":"10.1.2.3","enrollment":{"enabled":true,"mode":"token","token_required":true,"token_presented":false,"network_mode":"allowlist","network_allowed":false,"would_enroll":false,"reason":"AGENT_SOURCE_NOT_ALLOWED","detail":"rejected"},"credential":{"presented":false,"state":"absent"}}"#,
        );
        let config = insecure(url);
        let transport = Transport::new(&config).unwrap().unwrap();
        let preflight = transport.preflight().await.unwrap();
        assert_eq!(preflight.observed_source_ip, "10.1.2.3");
        assert!(!preflight.enrollment.would_enroll);
        assert_eq!(preflight.enrollment.reason, "AGENT_SOURCE_NOT_ALLOWED");
        assert_eq!(preflight.credential.state, "absent");
    }
}

use crate::config::ServerConfig;
use crate::model::Envelope;
use anyhow::{Context, Result};
use reqwest::{Certificate, Client, Identity};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug)]
pub struct AgentUnauthorized;

impl std::fmt::Display for AgentUnauthorized {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("server rejected the device credential")
    }
}

impl std::error::Error for AgentUnauthorized {}

pub fn is_unauthorized(error: &anyhow::Error) -> bool {
    error.downcast_ref::<AgentUnauthorized>().is_some()
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
}

#[derive(Debug, Deserialize)]
struct EnrollmentResponse {
    token: String,
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

    pub async fn enroll(
        &self,
        agent_id: &str,
        hostname: &str,
        claim_token: &str,
    ) -> Result<String> {
        let mut request = self
            .client
            .post(format!("{}/v1/agent/enroll", self.base_url))
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
            .context("request automatic enrollment")?;
        let status = response.status();
        anyhow::ensure!(
            status.is_success(),
            "automatic enrollment returned HTTP {status}"
        );
        let enrollment: EnrollmentResponse = response
            .json()
            .await
            .context("decode automatic enrollment response")?;
        anyhow::ensure!(
            enrollment.token.starts_with("ivq_at_"),
            "automatic enrollment returned an invalid device token"
        );
        Ok(enrollment.token)
    }

    pub async fn send(&self, envelope: &Envelope) -> Result<ServerAcknowledgement> {
        let url = format!("{}/v1/agent/events", self.base_url);
        let mut request = self
            .client
            .post(url)
            .header("X-Invenqor-Agent-Id", &envelope.agent_id)
            .header("X-Invenqor-Event-Id", &envelope.event_id)
            .json(envelope);
        if let Some(token) = &self.bearer_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.context("send event")?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AgentUnauthorized.into());
        }
        anyhow::ensure!(status.is_success(), "server returned HTTP {status}");
        let acknowledgement: ServerAcknowledgement = response
            .json()
            .await
            .context("decode server acknowledgement")?;
        anyhow::ensure!(acknowledgement.accepted, "server did not accept event");
        Ok(acknowledgement)
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

    #[tokio::test]
    async fn sends_identity_auth_and_decodes_acknowledgement() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
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
                    .unwrap();
                if request.len() >= headers_end + 4 + content_length {
                    break;
                }
            }
            let body = br#"{"accepted":true,"policy_version":"test-policy"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
            String::from_utf8(request).unwrap()
        });

        let config = ServerConfig {
            url: Some(format!("http://{address}")),
            bearer_token: Some("device-token".into()),
            enrollment_token: None,
            enrollment_token_file: None,
            ca_file: None,
            client_identity_pem: None,
            allow_insecure_http: true,
            timeout_seconds: 2,
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
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
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
                    .unwrap();
                if request.len() >= headers_end + 4 + content_length {
                    break;
                }
            }
            let body = br#"{"token":"ivq_at_device-token"}"#;
            write!(
                stream,
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
            String::from_utf8(request).unwrap()
        });
        let config = ServerConfig {
            url: Some(format!("http://{address}")),
            allow_insecure_http: true,
            timeout_seconds: 2,
            ..ServerConfig::default()
        };
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
}

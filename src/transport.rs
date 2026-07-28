use crate::config::ServerConfig;
use crate::model::Envelope;
use anyhow::{Context, Result};
use reqwest::{Certificate, Client, Identity};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct Transport {
    client: Client,
    base_url: String,
    bearer_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServerAcknowledgement {
    pub accepted: bool,
    #[serde(default)]
    pub policy_version: Option<String>,
}

impl Transport {
    pub fn new(config: &ServerConfig) -> Result<Option<Self>> {
        let Some(base_url) = config.url.as_ref() else {
            return Ok(None);
        };
        let mut builder = Client::builder()
            .https_only(!cfg!(debug_assertions))
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
        }))
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
            ca_file: None,
            client_identity_pem: None,
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
}

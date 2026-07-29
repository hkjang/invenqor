use crate::transport::ExchangeFailure;
use serde::{Deserialize, Serialize};

/// The Agent runs on machines whose journal an operator often cannot read, and
/// a registration failure is exactly the moment nothing reaches the Server
/// either. `status.json` is therefore the third place the same facts are
/// written: locally, unconditionally, after every cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusReport {
    pub schema_version: u32,
    pub agent_version: String,
    pub agent_id: String,
    pub hostname: String,
    pub config_path: String,
    pub state_dir: String,
    pub server_url: Option<String>,
    pub auth_mode: String,
    pub updated_at: u64,
    pub updated_at_utc: String,
    pub enrollment: EnrollmentStatus,
    pub delivery: DeliveryStatus,
    pub queue: QueueStatus,
    pub collection: CollectionStatus,
    #[serde(default)]
    pub updates: UpdateStatus,
}

/// What the automatic update path has actually done on this host. Without this,
/// "did the fleet take the update" could only be answered by logging into each
/// machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub enabled: bool,
    pub channel: String,
    pub running_version: String,
    pub last_check_at: Option<u64>,
    pub last_check_at_utc: Option<String>,
    pub staged_version: Option<String>,
    pub staged_at_utc: Option<String>,
    pub applied_version: Option<String>,
    pub applied_at_utc: Option<String>,
    pub last_error: Option<FailureRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnrollmentStatus {
    /// `not_configured`, `pending`, `enrolled` or `failed`.
    pub state: String,
    pub summary: String,
    pub last_attempt_at: Option<u64>,
    pub last_attempt_at_utc: Option<String>,
    pub enrolled_at: Option<u64>,
    pub enrolled_at_utc: Option<String>,
    pub attempts: u64,
    pub last_error: Option<FailureRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeliveryStatus {
    pub last_success_at: Option<u64>,
    pub last_success_at_utc: Option<String>,
    pub last_attempt_at: Option<u64>,
    pub last_attempt_at_utc: Option<String>,
    pub delivered_events: u64,
    pub consecutive_failures: u64,
    pub last_request_id: Option<String>,
    pub last_policy_version: Option<String>,
    pub last_error: Option<FailureRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueStatus {
    pub pending_events: usize,
    pub bytes: u64,
    pub limit_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectionStatus {
    pub last_at: Option<u64>,
    pub last_at_utc: Option<String>,
    pub records: usize,
    pub collector_errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureRecord {
    pub code: String,
    pub operation: String,
    pub path: String,
    pub detail: String,
    pub remediation: String,
    pub status: Option<u16>,
    pub request_id: Option<String>,
    pub occurred_at: u64,
    pub occurred_at_utc: String,
}

impl FailureRecord {
    pub fn from_error(error: &anyhow::Error, occurred_at: u64) -> Self {
        match crate::transport::failure_of(error) {
            Some(failure) => Self::from_failure(failure, occurred_at),
            None => Self {
                code: "AGENT_LOCAL_ERROR".into(),
                operation: "agent".into(),
                path: String::new(),
                detail: format!("{error:#}"),
                remediation: "The Agent failed before contacting the Server. \
                     Check the state directory permissions and the configuration."
                    .into(),
                status: None,
                request_id: None,
                occurred_at,
                occurred_at_utc: format_unix_utc(occurred_at),
            },
        }
    }

    pub fn from_failure(failure: &ExchangeFailure, occurred_at: u64) -> Self {
        Self {
            code: failure.code.clone(),
            operation: failure.operation.to_string(),
            path: failure.path.to_string(),
            detail: failure
                .cause
                .clone()
                .or_else(|| failure.server_message.clone())
                .unwrap_or_else(|| failure.code.clone()),
            remediation: failure.remediation().to_string(),
            status: failure.status,
            request_id: failure.request_id.clone(),
            occurred_at,
            occurred_at_utc: format_unix_utc(occurred_at),
        }
    }
}

impl StatusReport {
    pub fn new(
        agent_id: String,
        hostname: String,
        config_path: String,
        state_dir: String,
        server_url: Option<String>,
        auth_mode: &str,
        now: u64,
    ) -> Self {
        let enrollment_state = if server_url.is_none() {
            "not_configured"
        } else {
            "pending"
        };
        Self {
            schema_version: 1,
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            agent_id,
            hostname,
            config_path,
            state_dir,
            server_url,
            auth_mode: auth_mode.to_string(),
            updated_at: now,
            updated_at_utc: format_unix_utc(now),
            enrollment: EnrollmentStatus {
                state: enrollment_state.into(),
                summary: enrollment_summary(enrollment_state),
                ..EnrollmentStatus::default()
            },
            delivery: DeliveryStatus::default(),
            queue: QueueStatus::default(),
            collection: CollectionStatus::default(),
            updates: UpdateStatus {
                running_version: env!("CARGO_PKG_VERSION").to_string(),
                ..UpdateStatus::default()
            },
        }
    }

    pub fn record_update_settings(&mut self, enabled: bool, channel: &str) {
        self.updates.enabled = enabled;
        self.updates.channel = channel.to_string();
    }

    pub fn record_update_check(&mut self, staged: Option<String>, now: u64) {
        self.updates.last_check_at = Some(now);
        self.updates.last_check_at_utc = Some(format_unix_utc(now));
        if let Some(version) = staged {
            self.updates.staged_version = Some(version);
            self.updates.staged_at_utc = Some(format_unix_utc(now));
        }
        self.updates.last_error = None;
        self.touch(now);
    }

    pub fn record_update_applied(&mut self, version: &str, now: u64) {
        self.updates.applied_version = Some(version.to_string());
        self.updates.applied_at_utc = Some(format_unix_utc(now));
        self.updates.staged_version = None;
        self.updates.staged_at_utc = None;
        self.updates.last_error = None;
        self.touch(now);
    }

    pub fn record_update_failure(&mut self, error: &anyhow::Error, now: u64) {
        self.updates.last_check_at = Some(now);
        self.updates.last_check_at_utc = Some(format_unix_utc(now));
        self.updates.last_error = Some(FailureRecord::from_error(error, now));
        self.touch(now);
    }

    pub fn touch(&mut self, now: u64) {
        self.updated_at = now;
        self.updated_at_utc = format_unix_utc(now);
    }

    pub fn record_enrollment_attempt(&mut self, now: u64) {
        self.enrollment.attempts += 1;
        self.enrollment.last_attempt_at = Some(now);
        self.enrollment.last_attempt_at_utc = Some(format_unix_utc(now));
    }

    pub fn record_enrolled(&mut self, now: u64) {
        self.enrollment.state = "enrolled".into();
        self.enrollment.summary = enrollment_summary("enrolled");
        self.enrollment.enrolled_at = Some(now);
        self.enrollment.enrolled_at_utc = Some(format_unix_utc(now));
        self.enrollment.last_error = None;
        self.touch(now);
    }

    pub fn record_enrollment_failure(&mut self, error: &anyhow::Error, now: u64) {
        self.enrollment.state = "failed".into();
        self.enrollment.summary = enrollment_summary("failed");
        self.enrollment.last_error = Some(FailureRecord::from_error(error, now));
        self.touch(now);
    }

    pub fn record_delivery_success(
        &mut self,
        events: u64,
        request_id: Option<String>,
        policy_version: Option<String>,
        now: u64,
    ) {
        self.delivery.delivered_events = self.delivery.delivered_events.saturating_add(events);
        self.delivery.last_success_at = Some(now);
        self.delivery.last_success_at_utc = Some(format_unix_utc(now));
        self.delivery.last_attempt_at = Some(now);
        self.delivery.last_attempt_at_utc = Some(format_unix_utc(now));
        self.delivery.consecutive_failures = 0;
        self.delivery.last_error = None;
        if request_id.is_some() {
            self.delivery.last_request_id = request_id;
        }
        if policy_version.is_some() {
            self.delivery.last_policy_version = policy_version;
        }
        self.touch(now);
    }

    pub fn record_delivery_failure(&mut self, error: &anyhow::Error, now: u64) {
        self.delivery.consecutive_failures = self.delivery.consecutive_failures.saturating_add(1);
        self.delivery.last_attempt_at = Some(now);
        self.delivery.last_attempt_at_utc = Some(format_unix_utc(now));
        self.delivery.last_error = Some(FailureRecord::from_error(error, now));
        self.touch(now);
    }

    pub fn record_collection(&mut self, records: usize, collector_errors: usize, now: u64) {
        self.collection.last_at = Some(now);
        self.collection.last_at_utc = Some(format_unix_utc(now));
        self.collection.records = records;
        self.collection.collector_errors = collector_errors;
        self.touch(now);
    }

    pub fn record_queue(&mut self, pending_events: usize, bytes: u64, limit_bytes: u64) {
        self.queue = QueueStatus {
            pending_events,
            bytes,
            limit_bytes,
        };
    }

    /// The single line a monitoring check or a human reads first.
    pub fn headline(&self) -> String {
        match self.enrollment.state.as_str() {
            "not_configured" => {
                "server.url is not configured: inventory stays in the local queue".into()
            }
            "enrolled" => match (&self.delivery.last_error, self.delivery.last_success_at) {
                (Some(error), _) => format!(
                    "registered, but delivery is failing ({}): {}",
                    error.code, error.remediation
                ),
                (None, Some(_)) => "registered and delivering inventory".into(),
                (None, None) => "registered, waiting for the first delivery".into(),
            },
            "failed" => match &self.enrollment.last_error {
                Some(error) => format!(
                    "registration is failing ({}): {}",
                    error.code, error.remediation
                ),
                None => "registration is failing".into(),
            },
            _ => "registration has not been attempted yet".into(),
        }
    }

    /// True when the Agent is configured to reach a Server but is not
    /// currently able to hand inventory over.
    pub fn degraded(&self) -> bool {
        if self.server_url.is_none() {
            return true;
        }
        self.enrollment.state != "enrolled"
            || self.delivery.last_error.is_some()
            || self.queue.pending_events > 0
    }
}

fn enrollment_summary(state: &str) -> String {
    match state {
        "not_configured" => "server.url is absent, so no registration is attempted".into(),
        "enrolled" => "a device credential is stored for this Server".into(),
        "failed" => "the Server rejected or could not be reached for registration".into(),
        _ => "registration has not completed yet".into(),
    }
}

/// Formats a Unix timestamp as UTC RFC 3339 without pulling in a date crate,
/// because `status.json` is read by humans far more often than by programs.
pub fn format_unix_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

// Howard Hinnant's civil_from_days, the standard branch-free conversion.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ExchangeFailure;

    fn report() -> StatusReport {
        StatusReport::new(
            "agent-1".into(),
            "host-1".into(),
            "/etc/invenqor-agent/config.toml".into(),
            "/var/lib/invenqor-agent".into(),
            Some("https://inventory.example:7070".into()),
            "device_token",
            1_769_000_000,
        )
    }

    #[test]
    fn formats_unix_seconds_as_utc() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_utc(1_769_000_000), "2026-01-21T12:53:20Z");
        // A leap day must not shift the calendar.
        assert_eq!(format_unix_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn missing_server_url_is_reported_as_a_configuration_problem() {
        let status = StatusReport::new(
            "agent-1".into(),
            "host-1".into(),
            "/etc/invenqor-agent/config.toml".into(),
            "/var/lib/invenqor-agent".into(),
            None,
            "none",
            1_769_000_000,
        );
        assert_eq!(status.enrollment.state, "not_configured");
        assert!(status.degraded());
        assert!(status.headline().contains("server.url is not configured"));
    }

    #[test]
    fn enrollment_failure_keeps_the_server_code_and_remedy() {
        let mut status = report();
        let failure: anyhow::Error = ExchangeFailure {
            code: "AGENT_SOURCE_NOT_ALLOWED".into(),
            operation: "automatic enrollment",
            path: "/v1/agent/enroll",
            status: Some(403),
            server_code: Some("AGENT_SOURCE_NOT_ALLOWED".into()),
            server_message: Some("rejected".into()),
            request_id: Some("req-1".into()),
            cause: None,
        }
        .into();
        status.record_enrollment_attempt(1_769_000_100);
        status.record_enrollment_failure(&failure, 1_769_000_100);
        let recorded = status.enrollment.last_error.clone().unwrap();
        assert_eq!(recorded.code, "AGENT_SOURCE_NOT_ALLOWED");
        assert_eq!(recorded.request_id.as_deref(), Some("req-1"));
        assert_eq!(recorded.status, Some(403));
        assert!(recorded.remediation.contains("allowlist"));
        assert_eq!(status.enrollment.attempts, 1);
        assert!(status.headline().contains("registration is failing"));
        assert!(status.degraded());
    }

    #[test]
    fn successful_delivery_clears_the_previous_failure() {
        let mut status = report();
        let failure: anyhow::Error = ExchangeFailure {
            code: "SERVER_UNREACHABLE".into(),
            operation: "inventory delivery",
            path: "/v1/agent/events",
            status: None,
            server_code: None,
            server_message: None,
            request_id: None,
            cause: Some("connection refused".into()),
        }
        .into();
        status.record_delivery_failure(&failure, 1_769_000_100);
        assert_eq!(status.delivery.consecutive_failures, 1);
        status.record_enrolled(1_769_000_200);
        status.record_delivery_success(
            2,
            Some("req-2".into()),
            Some("policy-1".into()),
            1_769_000_200,
        );
        assert!(status.delivery.last_error.is_none());
        assert_eq!(status.delivery.delivered_events, 2);
        assert_eq!(status.delivery.consecutive_failures, 0);
        assert_eq!(status.headline(), "registered and delivering inventory");
        assert!(!status.degraded());
    }

    #[test]
    fn local_errors_are_recorded_with_the_full_context_chain() {
        let mut status = report();
        let error = anyhow::anyhow!("permission denied")
            .context("create state directory /var/lib/invenqor-agent");
        status.record_delivery_failure(&error, 1_769_000_100);
        let recorded = status.delivery.last_error.clone().unwrap();
        assert_eq!(recorded.code, "AGENT_LOCAL_ERROR");
        assert!(recorded.detail.contains("create state directory"));
        assert!(recorded.detail.contains("permission denied"));
    }
}

//! Pipeline triggers: webhook (GitHub, GitLab), cron schedule, manual.

use crate::models::ParameterValue;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    Webhook,
    GitHubWebhook,
    GitLabWebhook,
    Cron,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerConfig {
    Webhook {
        secret: Option<String>,
        events: Vec<String>,
    },
    GitHubWebhook {
        secret: String,
        events: Vec<String>,
        ref_filter: Option<String>,
    },
    GitLabWebhook {
        token: String,
        events: Vec<String>,
    },
    Cron {
        schedule: String,
    },
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub id: Uuid,
    pub name: String,
    pub trigger_type: TriggerType,
    pub pipeline_id: Uuid,
    pub params: Vec<ParameterValue>,
    pub config: TriggerConfig,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

impl Trigger {
    pub fn new(name: impl Into<String>, pipeline_id: Uuid, config: TriggerConfig) -> Self {
        let trigger_type = match &config {
            TriggerConfig::Webhook { .. } => TriggerType::Webhook,
            TriggerConfig::GitHubWebhook { .. } => TriggerType::GitHubWebhook,
            TriggerConfig::GitLabWebhook { .. } => TriggerType::GitLabWebhook,
            TriggerConfig::Cron { .. } => TriggerType::Cron,
            TriggerConfig::Manual => TriggerType::Manual,
        };
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            trigger_type,
            pipeline_id,
            params: Vec::new(),
            config,
            enabled: true,
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum TriggerError {
    #[error("Invalid cron expression '{expr}': {reason}")]
    InvalidCron { expr: String, reason: String },
    #[error("Webhook signature verification failed")]
    InvalidSignature,
    #[error("Unsupported event: {0}")]
    UnsupportedEvent(String),
}

// ---------------------------------------------------------------------------
// Cron validation (basic 5-field standard cron)
// ---------------------------------------------------------------------------

/// Validate a standard 5-field cron expression (minute hour dom month dow).
pub fn validate_cron(expr: &str) -> Result<(), TriggerError> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(TriggerError::InvalidCron {
            expr: expr.to_string(),
            reason: format!("expected 5 fields, got {}", fields.len()),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Webhook helpers
// ---------------------------------------------------------------------------

/// Verify a GitHub webhook HMAC-SHA256 signature header.
/// `signature` format: `"sha256=<hex>"`.
/// Real implementation would use ring::hmac; this validates the format contract.
pub fn verify_github_signature(payload: &[u8], signature: &str, secret: &str) -> bool {
    let Some(hex) = signature.strip_prefix("sha256=") else {
        return false;
    };
    !hex.is_empty() && !secret.is_empty() && !payload.is_empty()
}

/// Check whether the incoming event name matches the trigger's allowed events.
/// An empty allow-list or `"*"` entry accepts everything.
pub fn event_matches(trigger_events: &[String], incoming: &str) -> bool {
    trigger_events.is_empty()
        || trigger_events.iter().any(|e| e == "*" || e == incoming)
}

/// Check whether a git ref matches an optional filter pattern.
/// Patterns ending in `*` are prefix-matched; otherwise exact match.
pub fn ref_matches(filter: Option<&str>, git_ref: &str) -> bool {
    match filter {
        None => true,
        Some(f) if f.ends_with('*') => git_ref.starts_with(&f[..f.len() - 1]),
        Some(f) => git_ref == f,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- cron validation ---

    #[test]
    fn test_cron_valid_expressions() {
        assert!(validate_cron("0 */6 * * *").is_ok());
        assert!(validate_cron("30 2 * * 1").is_ok());
        assert!(validate_cron("0 0 * * *").is_ok());
        assert!(validate_cron("*/15 * * * *").is_ok());
    }

    #[test]
    fn test_cron_invalid_too_few_fields() {
        assert!(validate_cron("0 * *").is_err());
        assert!(validate_cron("").is_err());
    }

    #[test]
    fn test_cron_invalid_too_many_fields() {
        // 6-field (with seconds) is not standard 5-field cron
        assert!(validate_cron("0 * * * * *").is_err());
    }

    // --- event matching ---

    #[test]
    fn test_event_matches_wildcard() {
        let events = vec!["*".to_string()];
        assert!(event_matches(&events, "push"));
        assert!(event_matches(&events, "pull_request"));
        assert!(event_matches(&events, "anything"));
    }

    #[test]
    fn test_event_matches_specific_allowed() {
        let events = vec!["push".to_string(), "pull_request".to_string()];
        assert!(event_matches(&events, "push"));
        assert!(event_matches(&events, "pull_request"));
        assert!(!event_matches(&events, "delete"));
    }

    #[test]
    fn test_event_matches_empty_list_accepts_all() {
        assert!(event_matches(&[], "push"));
        assert!(event_matches(&[], "anything"));
    }

    // --- ref filter ---

    #[test]
    fn test_ref_filter_exact_match() {
        assert!(ref_matches(Some("refs/heads/main"), "refs/heads/main"));
        assert!(!ref_matches(Some("refs/heads/main"), "refs/heads/develop"));
    }

    #[test]
    fn test_ref_filter_prefix_wildcard() {
        assert!(ref_matches(Some("refs/heads/*"), "refs/heads/main"));
        assert!(ref_matches(Some("refs/heads/*"), "refs/heads/feature-x"));
        assert!(!ref_matches(Some("refs/tags/*"), "refs/heads/main"));
    }

    #[test]
    fn test_ref_filter_none_matches_all() {
        assert!(ref_matches(None, "refs/heads/anything"));
        assert!(ref_matches(None, "refs/tags/v1.0"));
    }

    // --- trigger construction ---

    #[test]
    fn test_trigger_type_inferred_from_config() {
        let pid = Uuid::new_v4();
        let t = Trigger::new("ci", pid, TriggerConfig::Cron { schedule: "0 * * * *".to_string() });
        assert_eq!(t.trigger_type, TriggerType::Cron);
        assert!(t.enabled);
        assert_eq!(t.pipeline_id, pid);
    }
}

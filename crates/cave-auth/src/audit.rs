//! Audit logging for every auth decision.
//!
//! Every allow/deny is logged via `tracing` with structured fields so that
//! cave-logs (or any OpenTelemetry collector) can ingest them.  The
//! `AuditLogger` is intentionally simple — it emits structured log events;
//! shipping to a persistent store is the responsibility of the log pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

// ─── Event types ─────────────────────────────────────────────────────────────

/// The outcome of an authorization decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum AuthDecision {
    Allowed,
    Denied { reason: String },
}

/// A single audit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Stable event ID (UUID v4)
    pub event_id: Uuid,
    pub timestamp: DateTime<Utc>,
    /// The CAVE user involved (None for anonymous/failed auth)
    pub cave_uid: Option<Uuid>,
    /// Email of the actor, if known
    pub email: Option<String>,
    /// What the actor tried to do (e.g. "jwt_validate", "pat_validate",
    /// "cave-flags:write", "cave-incidents:manage")
    pub action: String,
    /// Target resource identifier
    pub resource: String,
    pub decision: AuthDecision,
    /// Client IP address, if available
    pub ip_address: Option<String>,
    /// Extra context (request path, module, project, etc.)
    pub details: serde_json::Value,
}

impl AuditEvent {
    /// Successful authentication event.
    pub fn auth_success(cave_uid: Uuid, action: &str) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            cave_uid: Some(cave_uid),
            email: None,
            action: action.to_string(),
            resource: "auth".to_string(),
            decision: AuthDecision::Allowed,
            ip_address: None,
            details: serde_json::json!({}),
        }
    }

    /// Failed authentication event (no cave_uid available).
    pub fn auth_failure(action: &str, reason: &str) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            cave_uid: None,
            email: None,
            action: action.to_string(),
            resource: "auth".to_string(),
            decision: AuthDecision::Denied {
                reason: reason.to_string(),
            },
            ip_address: None,
            details: serde_json::json!({}),
        }
    }

    /// Authorization decision for a resource action.
    pub fn authz(
        cave_uid: Uuid,
        action: &str,
        resource: &str,
        allowed: bool,
        reason: Option<&str>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            cave_uid: Some(cave_uid),
            email: None,
            action: action.to_string(),
            resource: resource.to_string(),
            decision: if allowed {
                AuthDecision::Allowed
            } else {
                AuthDecision::Denied {
                    reason: reason.unwrap_or("insufficient_permissions").to_string(),
                }
            },
            ip_address: None,
            details: serde_json::json!({}),
        }
    }

    /// Attach an email address to this event.
    pub fn with_email(mut self, email: &str) -> Self {
        self.email = Some(email.to_string());
        self
    }

    /// Attach the client IP to this event.
    pub fn with_ip(mut self, ip: &str) -> Self {
        self.ip_address = Some(ip.to_string());
        self
    }

    /// Merge additional detail fields.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        if let (Some(existing), Some(new)) = (self.details.as_object_mut(), details.as_object()) {
            for (k, v) in new {
                existing.insert(k.clone(), v.clone());
            }
        }
        self
    }
}

// ─── Logger ──────────────────────────────────────────────────────────────────

/// Emits structured audit events via `tracing`.
///
/// In production the tracing subscriber ships these to cave-logs / the OTEL
/// collector; nothing else is needed here.
#[derive(Debug, Clone, Default)]
pub struct AuditLogger;

impl AuditLogger {
    pub fn new() -> Self {
        Self
    }

    /// Log an audit event.  Allowed decisions are INFO; denied are WARN.
    pub fn log(&self, event: AuditEvent) {
        let event_id = event.event_id;
        let cave_uid = event.cave_uid;
        let action = &event.action;
        let resource = &event.resource;
        let ip = event.ip_address.as_deref().unwrap_or("-");
        let email = event.email.as_deref().unwrap_or("-");

        match &event.decision {
            AuthDecision::Allowed => {
                info!(
                    event_id = %event_id,
                    cave_uid = ?cave_uid,
                    email = %email,
                    action = %action,
                    resource = %resource,
                    ip = %ip,
                    "cave_audit:allowed"
                );
            }
            AuthDecision::Denied { reason } => {
                warn!(
                    event_id = %event_id,
                    cave_uid = ?cave_uid,
                    email = %email,
                    action = %action,
                    resource = %resource,
                    reason = %reason,
                    ip = %ip,
                    "cave_audit:denied"
                );
            }
        }
    }

    /// Log an authz check inline — convenience wrapper.
    pub fn log_authz(
        &self,
        cave_uid: Uuid,
        action: &str,
        resource: &str,
        allowed: bool,
        reason: Option<&str>,
    ) {
        self.log(AuditEvent::authz(cave_uid, action, resource, allowed, reason));
    }
}

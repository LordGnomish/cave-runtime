//! Audit logging — immutable record of all auth events.
//!
//! Every authentication attempt, token operation, role change, and permission
//! denial is recorded for compliance and forensic purposes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// The type of audit event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    AuthAttempt,
    AuthSuccess,
    AuthFailure,
    TokenIssued,
    TokenRefreshed,
    TokenRevoked,
    TokenIntrospected,
    SessionCreated,
    SessionRefreshed,
    SessionInvalidated,
    RoleAssigned,
    RoleRevoked,
    PermissionChecked,
    PermissionDenied,
    PatCreated,
    PatRevoked,
    PatUsed,
    ScimUserCreated,
    ScimUserUpdated,
    ScimUserDeleted,
    ScimGroupCreated,
    ScimGroupUpdated,
    ScimGroupDeleted,
    TenantCreated,
    TenantSuspended,
    MemberAdded,
    MemberRemoved,
}

impl AuditEventType {
    pub fn is_security_sensitive(&self) -> bool {
        matches!(
            self,
            Self::AuthFailure
                | Self::PermissionDenied
                | Self::TokenRevoked
                | Self::SessionInvalidated
                | Self::RoleRevoked
                | Self::PatRevoked
                | Self::TenantSuspended
        )
    }
}

/// Outcome of an audited action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Success,
    Failure,
    Partial,
}

/// A single immutable audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub outcome: AuditOutcome,
    /// User who performed the action (None for anonymous/system).
    pub actor_id: Option<Uuid>,
    pub tenant_id: String,
    /// The resource being acted upon.
    pub resource_type: String,
    pub resource_id: Option<String>,
    /// Short description of the action.
    pub action: String,
    /// Additional structured data.
    pub details: serde_json::Value,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub session_id: Option<Uuid>,
    pub request_id: Option<String>,
}

impl AuditEvent {
    pub fn new(
        event_type: AuditEventType,
        outcome: AuditOutcome,
        tenant_id: &str,
        action: &str,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            event_type,
            outcome,
            actor_id: None,
            tenant_id: tenant_id.to_string(),
            resource_type: String::new(),
            resource_id: None,
            action: action.to_string(),
            details: serde_json::Value::Null,
            ip_address: None,
            user_agent: None,
            session_id: None,
            request_id: None,
        }
    }

    pub fn with_actor(mut self, actor_id: Uuid) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    pub fn with_resource(mut self, resource_type: &str, resource_id: Option<&str>) -> Self {
        self.resource_type = resource_type.to_string();
        self.resource_id = resource_id.map(|s| s.to_string());
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    pub fn with_ip(mut self, ip: &str) -> Self {
        self.ip_address = Some(ip.to_string());
        self
    }

    pub fn with_session(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }
}

/// Query filters for audit log retrieval.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub tenant_id: Option<String>,
    pub actor_id: Option<Uuid>,
    pub event_type: Option<AuditEventType>,
    pub resource_type: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
    pub security_only: bool,
}

/// Audit log — append-only storage with query support.
#[derive(Clone)]
pub struct AuditLog {
    events: Arc<RwLock<Vec<AuditEvent>>>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Record an audit event.
    pub async fn record(&self, event: AuditEvent) {
        if event.event_type.is_security_sensitive() {
            tracing::warn!(
                event_type = ?event.event_type,
                tenant = %event.tenant_id,
                actor = ?event.actor_id,
                action = %event.action,
                "Security-sensitive audit event"
            );
        } else {
            tracing::debug!(
                event_type = ?event.event_type,
                tenant = %event.tenant_id,
                action = %event.action,
                "Audit event"
            );
        }
        self.events.write().await.push(event);
    }

    /// Query audit events with optional filters.
    pub async fn query(&self, q: &AuditQuery) -> Vec<AuditEvent> {
        let events = self.events.read().await;
        let limit = q.limit.unwrap_or(1000);

        events
            .iter()
            .rev() // Most recent first
            .filter(|e| {
                if let Some(ref tid) = q.tenant_id {
                    if &e.tenant_id != tid {
                        return false;
                    }
                }
                if let Some(actor) = q.actor_id {
                    if e.actor_id != Some(actor) {
                        return false;
                    }
                }
                if let Some(ref et) = q.event_type {
                    if &e.event_type != et {
                        return false;
                    }
                }
                if let Some(ref rt) = q.resource_type {
                    if &e.resource_type != rt {
                        return false;
                    }
                }
                if let Some(from) = q.from {
                    if e.timestamp < from {
                        return false;
                    }
                }
                if let Some(to) = q.to {
                    if e.timestamp > to {
                        return false;
                    }
                }
                if q.security_only && !e.event_type.is_security_sensitive() {
                    return false;
                }
                true
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub async fn count(&self) -> usize {
        self.events.read().await.len()
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audit_log_records_event() {
        let log = AuditLog::new();
        let event = AuditEvent::new(
            AuditEventType::AuthSuccess,
            AuditOutcome::Success,
            "acme",
            "user login",
        )
        .with_actor(Uuid::new_v4())
        .with_ip("10.0.0.1");

        log.record(event).await;
        assert_eq!(log.count().await, 1);
    }

    #[tokio::test]
    async fn audit_log_query_by_tenant() {
        let log = AuditLog::new();

        log.record(AuditEvent::new(
            AuditEventType::AuthSuccess,
            AuditOutcome::Success,
            "tenant-a",
            "login",
        ))
        .await;
        log.record(AuditEvent::new(
            AuditEventType::AuthSuccess,
            AuditOutcome::Success,
            "tenant-b",
            "login",
        ))
        .await;

        let results = log
            .query(&AuditQuery {
                tenant_id: Some("tenant-a".to_string()),
                ..Default::default()
            })
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tenant_id, "tenant-a");
    }

    #[tokio::test]
    async fn audit_log_security_sensitive_filter() {
        let log = AuditLog::new();

        log.record(AuditEvent::new(
            AuditEventType::AuthSuccess,
            AuditOutcome::Success,
            "acme",
            "login",
        ))
        .await;
        log.record(AuditEvent::new(
            AuditEventType::PermissionDenied,
            AuditOutcome::Failure,
            "acme",
            "denied",
        ))
        .await;
        log.record(AuditEvent::new(
            AuditEventType::TokenRevoked,
            AuditOutcome::Success,
            "acme",
            "revoke",
        ))
        .await;

        let sensitive = log
            .query(&AuditQuery {
                security_only: true,
                ..Default::default()
            })
            .await;

        assert_eq!(sensitive.len(), 2);
        assert!(sensitive
            .iter()
            .all(|e| e.event_type.is_security_sensitive()));
    }
}

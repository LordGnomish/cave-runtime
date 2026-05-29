// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ── Core Incident Models ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Incident {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub severity: IncidentSeverity,
    pub status: IncidentStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_by: Uuid,
    pub assigned_to: Option<Uuid>,
    pub timeline: Vec<TimelineEntry>,
    pub tags: Vec<String>,
    pub responders: Vec<Responder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IncidentSeverity {
    P1,
    P2,
    P3,
    P4,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Open,
    Acknowledged,
    Resolved,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineEntry {
    pub id: Uuid,
    pub incident_id: Uuid,
    pub event_type: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub created_by: Uuid,
}

// ── Responder Models ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Responder {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: ResponderRole,
    pub paged_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponderRole {
    CommandingOfficer,
    Responder,
    Observer,
}

// ── PostMortem Models ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PostMortem {
    pub id: Uuid,
    pub incident_id: Uuid,
    pub title: String,
    pub summary: String,
    pub root_cause: String,
    pub action_items: Vec<String>,
    pub status: PostMortemStatus,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub author_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PostMortemStatus {
    Draft,
    InReview,
    Published,
}

// ── Incident Metrics ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IncidentMetrics {
    pub total_incidents: u64,
    pub open_incidents: u64,
    pub p1_count: u64,
    pub p2_count: u64,
    pub p3_count: u64,
    pub p4_count: u64,
    /// Mean time to acknowledge in seconds (None if no resolved incidents)
    pub avg_time_to_acknowledge_secs: Option<f64>,
    /// Mean time to resolve in seconds (None if no resolved incidents)
    pub avg_time_to_resolve_secs: Option<f64>,
    pub resolved_this_week: u64,
}

// ── On-Call Schedule Models ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OnCallUser {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub notification_prefs: Vec<NotificationChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    Email,
    Slack,
    Sms,
    PagerDuty,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleLayer {
    pub id: Uuid,
    pub name: String,
    pub rotation_type: RotationType,
    pub rotation_period_days: u32,
    pub users: Vec<OnCallUser>,
    pub current_index: usize,
    pub starts_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RotationType {
    Daily,
    Weekly,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OnCallSchedule {
    pub id: Uuid,
    pub name: String,
    pub timezone: String,
    pub layers: Vec<ScheduleLayer>,
}

// ── Escalation Policy Models ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EscalationPolicy {
    pub id: Uuid,
    pub name: String,
    pub steps: Vec<EscalationStep>,
    pub repeat_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EscalationStep {
    pub delay_minutes: u32,
    pub targets: Vec<EscalationTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type", content = "id")]
pub enum EscalationTarget {
    User(Uuid),
    Schedule(Uuid),
    Team(String),
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum IncidentError {
    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: IncidentStatus,
        to: IncidentStatus,
    },
    #[error("Incident is already resolved")]
    AlreadyResolved,
    #[error("Incident must be acknowledged before resolving")]
    NotAcknowledged,
    #[error("Incident not found: {0}")]
    NotFound(Uuid),
    #[error("PostMortem not found: {0}")]
    PostMortemNotFound(Uuid),
}

// ── Requests ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CreateIncidentRequest {
    pub title: String,
    pub description: String,
    pub severity: IncidentSeverity,
    pub created_by: Uuid,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcknowledgeRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveRequest {
    pub user_id: Uuid,
    pub resolution: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloseRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePostMortemRequest {
    pub incident_id: Uuid,
    pub title: String,
    pub summary: String,
    pub root_cause: String,
    pub action_items: Vec<String>,
    pub author_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddResponderRequest {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: ResponderRole,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_incident() -> Incident {
        let now = Utc::now();
        Incident {
            id: Uuid::new_v4(),
            title: "Database down".to_string(),
            description: "Primary DB is unreachable".to_string(),
            severity: IncidentSeverity::P1,
            status: IncidentStatus::Open,
            created_at: now,
            updated_at: now,
            acknowledged_at: None,
            resolved_at: None,
            created_by: Uuid::new_v4(),
            assigned_to: None,
            timeline: vec![],
            tags: vec!["database".to_string(), "infra".to_string()],
            responders: vec![],
        }
    }

    #[test]
    fn test_incident_serde_roundtrip() {
        let incident = make_incident();
        let json = serde_json::to_string(&incident).unwrap();
        let restored: Incident = serde_json::from_str(&json).unwrap();
        assert_eq!(incident, restored);
    }

    #[test]
    fn test_incident_severity_serde() {
        for (variant, expected) in [
            (IncidentSeverity::P1, "\"p1\""),
            (IncidentSeverity::P2, "\"p2\""),
            (IncidentSeverity::P3, "\"p3\""),
            (IncidentSeverity::P4, "\"p4\""),
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            assert_eq!(s, expected);
            let restored: IncidentSeverity = serde_json::from_str(&s).unwrap();
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn test_incident_status_serde() {
        for (variant, expected) in [
            (IncidentStatus::Open, "\"open\""),
            (IncidentStatus::Acknowledged, "\"acknowledged\""),
            (IncidentStatus::Resolved, "\"resolved\""),
            (IncidentStatus::Closed, "\"closed\""),
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            assert_eq!(s, expected);
            let restored: IncidentStatus = serde_json::from_str(&s).unwrap();
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn test_timeline_entry_serde_roundtrip() {
        let incident_id = Uuid::new_v4();
        let entry = TimelineEntry {
            id: Uuid::new_v4(),
            incident_id,
            event_type: "created".to_string(),
            message: "Incident opened".to_string(),
            created_at: Utc::now(),
            created_by: Uuid::new_v4(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let restored: TimelineEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, restored);
    }

    #[test]
    fn test_incident_with_timestamps_serde() {
        let mut incident = make_incident();
        incident.status = IncidentStatus::Resolved;
        incident.acknowledged_at = Some(Utc::now());
        incident.resolved_at = Some(Utc::now() + chrono::Duration::minutes(30));
        let json = serde_json::to_string(&incident).unwrap();
        let restored: Incident = serde_json::from_str(&json).unwrap();
        assert_eq!(incident, restored);
        assert!(restored.acknowledged_at.is_some());
        assert!(restored.resolved_at.is_some());
    }

    #[test]
    fn test_incident_tags_preserved() {
        let incident = make_incident();
        let json = serde_json::to_string(&incident).unwrap();
        let restored: Incident = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.tags, vec!["database", "infra"]);
    }
}

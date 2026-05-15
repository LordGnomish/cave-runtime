// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Incident {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub severity: IncidentSeverity,
    pub status: IncidentStatus,
    pub created_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_by: Uuid,
    pub assigned_to: Option<Uuid>,
    pub timeline: Vec<TimelineEntry>,
    pub tags: Vec<String>,
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
}

#[derive(Debug, Deserialize)]
pub struct CreateIncidentRequest {
    pub title: String,
    pub description: String,
    pub severity: IncidentSeverity,
    pub created_by: Uuid,
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_incident() -> Incident {
        Incident {
            id: Uuid::new_v4(),
            title: "Database down".to_string(),
            description: "Primary DB is unreachable".to_string(),
            severity: IncidentSeverity::P1,
            status: IncidentStatus::Open,
            created_at: Utc::now(),
            acknowledged_at: None,
            resolved_at: None,
            created_by: Uuid::new_v4(),
            assigned_to: None,
            timeline: vec![],
            tags: vec!["database".to_string(), "infra".to_string()],
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

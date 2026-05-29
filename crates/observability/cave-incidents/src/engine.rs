// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{
    CreateIncidentRequest, Incident, IncidentError, IncidentMetrics, IncidentSeverity,
    IncidentStatus, TimelineEntry,
};
use chrono::Utc;
use uuid::Uuid;

pub fn create_incident(req: CreateIncidentRequest) -> Incident {
    let now = Utc::now();
    let id = Uuid::new_v4();
    let entry = TimelineEntry {
        id: Uuid::new_v4(),
        incident_id: id,
        event_type: "created".to_string(),
        message: format!("Incident created: {}", req.title),
        created_at: now,
        created_by: req.created_by,
    };
    Incident {
        id,
        title: req.title,
        description: req.description,
        severity: req.severity,
        status: IncidentStatus::Open,
        created_at: now,
        updated_at: now,
        acknowledged_at: None,
        resolved_at: None,
        created_by: req.created_by,
        assigned_to: None,
        timeline: vec![entry],
        tags: req.tags,
        responders: vec![],
    }
}

pub fn acknowledge(incident: &mut Incident, user_id: Uuid) -> Result<(), IncidentError> {
    match incident.status {
        IncidentStatus::Open => {
            let now = Utc::now();
            incident.status = IncidentStatus::Acknowledged;
            incident.acknowledged_at = Some(now);
            incident.updated_at = now;
            incident.timeline.push(TimelineEntry {
                id: Uuid::new_v4(),
                incident_id: incident.id,
                event_type: "acknowledged".to_string(),
                message: format!("Acknowledged by {user_id}"),
                created_at: now,
                created_by: user_id,
            });
            Ok(())
        }
        IncidentStatus::Resolved | IncidentStatus::Closed => Err(IncidentError::AlreadyResolved),
        _ => Err(IncidentError::InvalidTransition {
            from: incident.status.clone(),
            to: IncidentStatus::Acknowledged,
        }),
    }
}

pub fn resolve(
    incident: &mut Incident,
    user_id: Uuid,
    resolution: String,
) -> Result<(), IncidentError> {
    match incident.status {
        IncidentStatus::Acknowledged => {
            let now = Utc::now();
            incident.status = IncidentStatus::Resolved;
            incident.resolved_at = Some(now);
            incident.updated_at = now;
            incident.timeline.push(TimelineEntry {
                id: Uuid::new_v4(),
                incident_id: incident.id,
                event_type: "resolved".to_string(),
                message: resolution,
                created_at: now,
                created_by: user_id,
            });
            Ok(())
        }
        IncidentStatus::Open => Err(IncidentError::NotAcknowledged),
        IncidentStatus::Resolved | IncidentStatus::Closed => Err(IncidentError::AlreadyResolved),
    }
}

pub fn close(incident: &mut Incident, user_id: Uuid) -> Result<(), IncidentError> {
    match incident.status {
        IncidentStatus::Resolved => {
            let now = Utc::now();
            incident.status = IncidentStatus::Closed;
            incident.updated_at = now;
            incident.timeline.push(TimelineEntry {
                id: Uuid::new_v4(),
                incident_id: incident.id,
                event_type: "closed".to_string(),
                message: "Incident closed".to_string(),
                created_at: now,
                created_by: user_id,
            });
            Ok(())
        }
        IncidentStatus::Closed => Err(IncidentError::AlreadyResolved),
        _ => Err(IncidentError::InvalidTransition {
            from: incident.status.clone(),
            to: IncidentStatus::Closed,
        }),
    }
}

pub fn add_timeline_entry(incident: &mut Incident, event_type: &str, message: &str, user_id: Uuid) {
    let now = Utc::now();
    incident.timeline.push(TimelineEntry {
        id: Uuid::new_v4(),
        incident_id: incident.id,
        event_type: event_type.to_string(),
        message: message.to_string(),
        created_at: now,
        created_by: user_id,
    });
    incident.updated_at = now;
}

pub fn time_to_acknowledge_secs(incident: &Incident) -> Option<i64> {
    incident
        .acknowledged_at
        .map(|ack| (ack - incident.created_at).num_seconds())
}

pub fn time_to_resolve_secs(incident: &Incident) -> Option<i64> {
    match (incident.acknowledged_at, incident.resolved_at) {
        (Some(ack), Some(res)) => Some((res - ack).num_seconds()),
        _ => None,
    }
}

/// Compute aggregate metrics from a slice of incident references.
pub fn compute_metrics_from_refs(incidents: &[&Incident]) -> IncidentMetrics {
    let total = incidents.len() as u64;
    let open = incidents
        .iter()
        .filter(|i| matches!(i.status, IncidentStatus::Open | IncidentStatus::Acknowledged))
        .count() as u64;

    let p1 = incidents.iter().filter(|i| matches!(i.severity, IncidentSeverity::P1)).count() as u64;
    let p2 = incidents.iter().filter(|i| matches!(i.severity, IncidentSeverity::P2)).count() as u64;
    let p3 = incidents.iter().filter(|i| matches!(i.severity, IncidentSeverity::P3)).count() as u64;
    let p4 = incidents.iter().filter(|i| matches!(i.severity, IncidentSeverity::P4)).count() as u64;

    // MTTA: mean seconds from created_at to acknowledged_at (resolved/closed only)
    let tta_values: Vec<f64> = incidents
        .iter()
        .filter_map(|i| time_to_acknowledge_secs(i))
        .map(|s| s as f64)
        .collect();
    let avg_tta = if tta_values.is_empty() {
        None
    } else {
        Some(tta_values.iter().sum::<f64>() / tta_values.len() as f64)
    };

    // MTTR: mean seconds from acknowledged_at to resolved_at
    let ttr_values: Vec<f64> = incidents
        .iter()
        .filter_map(|i| time_to_resolve_secs(i))
        .map(|s| s as f64)
        .collect();
    let avg_ttr = if ttr_values.is_empty() {
        None
    } else {
        Some(ttr_values.iter().sum::<f64>() / ttr_values.len() as f64)
    };

    // Resolved this week
    let one_week_ago = Utc::now() - chrono::Duration::days(7);
    let resolved_this_week = incidents
        .iter()
        .filter(|i| {
            matches!(i.status, IncidentStatus::Resolved | IncidentStatus::Closed)
                && i.resolved_at
                    .map(|t| t > one_week_ago)
                    .unwrap_or(false)
        })
        .count() as u64;

    IncidentMetrics {
        total_incidents: total,
        open_incidents: open,
        p1_count: p1,
        p2_count: p2,
        p3_count: p3,
        p4_count: p4,
        avg_time_to_acknowledge_secs: avg_tta,
        avg_time_to_resolve_secs: avg_ttr,
        resolved_this_week,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IncidentSeverity, IncidentStatus};

    fn make_request(title: &str) -> CreateIncidentRequest {
        CreateIncidentRequest {
            title: title.to_string(),
            description: "Something went wrong".to_string(),
            severity: IncidentSeverity::P2,
            created_by: Uuid::new_v4(),
            tags: vec!["infra".to_string()],
        }
    }

    #[test]
    fn test_create_incident_open_status() {
        let incident = create_incident(make_request("DB Outage"));
        assert_eq!(incident.status, IncidentStatus::Open);
    }

    #[test]
    fn test_create_incident_has_timeline_entry() {
        let incident = create_incident(make_request("DB Outage"));
        assert_eq!(incident.timeline.len(), 1);
        assert_eq!(incident.timeline[0].event_type, "created");
    }

    #[test]
    fn test_acknowledge_open_incident() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        assert_eq!(incident.status, IncidentStatus::Acknowledged);
    }

    #[test]
    fn test_acknowledge_sets_timestamp() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        assert!(incident.acknowledged_at.is_some());
    }

    #[test]
    fn test_acknowledge_adds_timeline() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        assert_eq!(incident.timeline.len(), 2);
        assert_eq!(incident.timeline[1].event_type, "acknowledged");
    }

    #[test]
    fn test_resolve_acknowledged_incident() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        resolve(
            &mut incident,
            user,
            "Fixed the DB connection pool".to_string(),
        )
        .unwrap();
        assert_eq!(incident.status, IncidentStatus::Resolved);
    }

    #[test]
    fn test_resolve_open_incident_fails() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        let result = resolve(&mut incident, user, "Resolved".to_string());
        assert!(matches!(result, Err(IncidentError::NotAcknowledged)));
    }

    #[test]
    fn test_resolve_twice_fails() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        resolve(&mut incident, user, "Fixed".to_string()).unwrap();
        let result = resolve(&mut incident, user, "Fixed again".to_string());
        assert!(matches!(result, Err(IncidentError::AlreadyResolved)));
    }

    #[test]
    fn test_full_lifecycle() {
        let mut incident = create_incident(make_request("Network Outage"));
        let user = Uuid::new_v4();
        assert_eq!(incident.status, IncidentStatus::Open);
        acknowledge(&mut incident, user).unwrap();
        assert_eq!(incident.status, IncidentStatus::Acknowledged);
        resolve(&mut incident, user, "Network restored".to_string()).unwrap();
        assert_eq!(incident.status, IncidentStatus::Resolved);
        assert!(incident.resolved_at.is_some());
        // timeline: created, acknowledged, resolved = 3 entries
        assert_eq!(incident.timeline.len(), 3);
    }

    #[test]
    fn test_close_resolved_incident() {
        let mut incident = create_incident(make_request("Network Outage"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        resolve(&mut incident, user, "Fixed".to_string()).unwrap();
        close(&mut incident, user).unwrap();
        assert_eq!(incident.status, IncidentStatus::Closed);
        assert_eq!(incident.timeline.len(), 4);
    }

    #[test]
    fn test_close_open_incident_fails() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        let result = close(&mut incident, user);
        assert!(matches!(result, Err(IncidentError::InvalidTransition { .. })));
    }

    #[test]
    fn test_add_timeline_entry() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        add_timeline_entry(&mut incident, "note", "Escalated to on-call", user);
        assert_eq!(incident.timeline.len(), 2);
        let entry = &incident.timeline[1];
        assert_eq!(entry.event_type, "note");
        assert_eq!(entry.message, "Escalated to on-call");
        assert_eq!(entry.created_by, user);
    }

    #[test]
    fn test_time_to_acknowledge_none_when_open() {
        let incident = create_incident(make_request("API Down"));
        assert!(time_to_acknowledge_secs(&incident).is_none());
    }

    #[test]
    fn test_time_to_resolve_none_when_not_resolved() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        assert!(time_to_resolve_secs(&incident).is_none());
    }

    #[test]
    fn test_acknowledge_already_resolved_fails() {
        let mut incident = create_incident(make_request("API Down"));
        let user = Uuid::new_v4();
        acknowledge(&mut incident, user).unwrap();
        resolve(&mut incident, user, "Done".to_string()).unwrap();
        let result = acknowledge(&mut incident, user);
        assert!(matches!(result, Err(IncidentError::AlreadyResolved)));
    }

    #[test]
    fn test_compute_metrics_empty() {
        let empty: Vec<&Incident> = vec![];
        let m = compute_metrics_from_refs(&empty);
        assert_eq!(m.total_incidents, 0);
        assert_eq!(m.open_incidents, 0);
        assert!(m.avg_time_to_acknowledge_secs.is_none());
    }

    #[test]
    fn test_compute_metrics_counts_by_severity() {
        let i1 = create_incident(CreateIncidentRequest {
            title: "P1".to_string(),
            description: "".to_string(),
            severity: IncidentSeverity::P1,
            created_by: Uuid::new_v4(),
            tags: vec![],
        });
        let i2 = create_incident(CreateIncidentRequest {
            title: "P2".to_string(),
            description: "".to_string(),
            severity: IncidentSeverity::P2,
            created_by: Uuid::new_v4(),
            tags: vec![],
        });
        let i3 = create_incident(CreateIncidentRequest {
            title: "P3".to_string(),
            description: "".to_string(),
            severity: IncidentSeverity::P3,
            created_by: Uuid::new_v4(),
            tags: vec![],
        });
        let m = compute_metrics_from_refs(&[&i1, &i2, &i3]);
        assert_eq!(m.total_incidents, 3);
        assert_eq!(m.p1_count, 1);
        assert_eq!(m.p2_count, 1);
        assert_eq!(m.p3_count, 1);
        assert_eq!(m.p4_count, 0);
        assert_eq!(m.open_incidents, 3);
    }
}

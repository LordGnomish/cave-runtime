use crate::models::{CreateIncidentRequest, Incident, IncidentError, IncidentStatus, TimelineEntry};
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
        acknowledged_at: None,
        resolved_at: None,
        created_by: req.created_by,
        assigned_to: None,
        timeline: vec![entry],
        tags: req.tags,
    }
}

pub fn acknowledge(incident: &mut Incident, user_id: Uuid) -> Result<(), IncidentError> {
    match incident.status {
        IncidentStatus::Open => {
            incident.status = IncidentStatus::Acknowledged;
            incident.acknowledged_at = Some(Utc::now());
            incident.timeline.push(TimelineEntry {
                id: Uuid::new_v4(),
                incident_id: incident.id,
                event_type: "acknowledged".to_string(),
                message: format!("Acknowledged by {user_id}"),
                created_at: Utc::now(),
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

pub fn resolve(incident: &mut Incident, user_id: Uuid, resolution: String) -> Result<(), IncidentError> {
    match incident.status {
        IncidentStatus::Acknowledged => {
            incident.status = IncidentStatus::Resolved;
            incident.resolved_at = Some(Utc::now());
            incident.timeline.push(TimelineEntry {
                id: Uuid::new_v4(),
                incident_id: incident.id,
                event_type: "resolved".to_string(),
                message: resolution,
                created_at: Utc::now(),
                created_by: user_id,
            });
            Ok(())
        }
        IncidentStatus::Open => Err(IncidentError::NotAcknowledged),
        IncidentStatus::Resolved | IncidentStatus::Closed => Err(IncidentError::AlreadyResolved),
        _ => Err(IncidentError::InvalidTransition {
            from: incident.status.clone(),
            to: IncidentStatus::Resolved,
        }),
    }
}

pub fn add_timeline_entry(incident: &mut Incident, event_type: &str, message: &str, user_id: Uuid) {
    incident.timeline.push(TimelineEntry {
        id: Uuid::new_v4(),
        incident_id: incident.id,
        event_type: event_type.to_string(),
        message: message.to_string(),
        created_at: Utc::now(),
        created_by: user_id,
    });
}

pub fn time_to_acknowledge_secs(incident: &Incident) -> Option<i64> {
    incident.acknowledged_at.map(|ack| (ack - incident.created_at).num_seconds())
}

pub fn time_to_resolve_secs(incident: &Incident) -> Option<i64> {
    match (incident.acknowledged_at, incident.resolved_at) {
        (Some(ack), Some(res)) => Some((res - ack).num_seconds()),
        _ => None,
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
        resolve(&mut incident, user, "Fixed the DB connection pool".to_string()).unwrap();
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
        // Trying to ack again after resolve should fail
        let result = acknowledge(&mut incident, user);
        assert!(matches!(result, Err(IncidentError::AlreadyResolved)));
    }
}

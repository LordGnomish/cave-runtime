// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PamSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub target: AccessTarget,
    pub session_type: SessionType,
    pub reason: String,
    pub approved_by: Option<Uuid>,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub recording_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessTarget {
    pub hostname: String,
    pub port: u16,
    pub target_type: TargetType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TargetType {
    Server,
    Database,
    Kubernetes,
    Application,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    Ssh,
    Rdp,
    Database,
    Kubectl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    PendingApproval,
    Active,
    Expired,
    Terminated,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessRequest {
    pub id: Uuid,
    pub user_id: Uuid,
    pub requested_roles: Vec<String>,
    pub reason: String,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: RequestStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_session(status: SessionStatus) -> PamSession {
        let now = Utc::now();
        PamSession {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            target: AccessTarget {
                hostname: "db-prod-01".to_string(),
                port: 5432,
                target_type: TargetType::Database,
            },
            session_type: SessionType::Database,
            reason: "Emergency access for incident".to_string(),
            approved_by: Some(Uuid::new_v4()),
            status,
            started_at: now,
            expires_at: now + Duration::hours(1),
            recording_path: None,
        }
    }

    #[test]
    fn test_session_status_serialization() {
        let statuses = vec![
            SessionStatus::PendingApproval,
            SessionStatus::Active,
            SessionStatus::Expired,
            SessionStatus::Terminated,
            SessionStatus::Denied,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let back: SessionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_session_roundtrip() {
        let session = make_session(SessionStatus::Active);
        let json = serde_json::to_string(&session).unwrap();
        let back: PamSession = serde_json::from_str(&json).unwrap();
        assert_eq!(session, back);
    }

    #[test]
    fn test_access_request_roundtrip() {
        let now = Utc::now();
        let req = AccessRequest {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            requested_roles: vec!["admin".to_string(), "db-read".to_string()],
            reason: "Debugging production issue".to_string(),
            requested_at: now,
            expires_at: now + chrono::Duration::hours(4),
            status: RequestStatus::Pending,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: AccessRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn test_target_type_serialization() {
        let json = serde_json::to_string(&TargetType::Kubernetes).unwrap();
        assert_eq!(json, "\"kubernetes\"");
    }

    #[test]
    fn test_request_status_denied_serializes() {
        let json = serde_json::to_string(&RequestStatus::Denied).unwrap();
        assert_eq!(json, "\"denied\"");
    }
}

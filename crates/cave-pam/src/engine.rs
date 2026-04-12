use crate::models::{AccessRequest, PamSession, RequestStatus, SessionStatus};
use chrono::Utc;

/// Check if a session is currently active and not expired
pub fn is_session_active(session: &PamSession) -> bool {
    session.status == SessionStatus::Active && Utc::now() < session.expires_at
}

/// Check if a session has expired by time (regardless of status)
pub fn is_session_expired(session: &PamSession) -> bool {
    Utc::now() > session.expires_at
}

/// Check if an access request is still valid (pending and not expired)
pub fn is_request_valid(request: &AccessRequest) -> bool {
    request.status == RequestStatus::Pending && Utc::now() < request.expires_at
}

/// Count active sessions for a user
pub fn active_session_count(sessions: &[PamSession], user_id: &uuid::Uuid) -> usize {
    sessions.iter()
        .filter(|s| &s.user_id == user_id && s.status == SessionStatus::Active)
        .count()
}

/// Filter sessions requiring approval
pub fn pending_sessions(sessions: &[PamSession]) -> Vec<&PamSession> {
    sessions.iter()
        .filter(|s| s.status == SessionStatus::PendingApproval)
        .collect()
}

/// Check if a session has a recording
pub fn is_recorded(session: &PamSession) -> bool {
    session.recording_path.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AccessTarget, SessionType, TargetType};
    use uuid::Uuid;
    use chrono::{Duration, Utc};

    fn make_session(
        user_id: Uuid,
        status: SessionStatus,
        expires_offset_secs: i64,
        recording_path: Option<String>,
    ) -> PamSession {
        let now = Utc::now();
        PamSession {
            id: Uuid::new_v4(),
            user_id,
            target: AccessTarget {
                hostname: "server-01".to_string(),
                port: 22,
                target_type: TargetType::Server,
            },
            session_type: SessionType::Ssh,
            reason: "Maintenance work".to_string(),
            approved_by: Some(Uuid::new_v4()),
            status,
            started_at: now,
            expires_at: now + Duration::seconds(expires_offset_secs),
            recording_path,
        }
    }

    fn make_request(status: RequestStatus, expires_offset_secs: i64) -> AccessRequest {
        let now = Utc::now();
        AccessRequest {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            requested_roles: vec!["admin".to_string()],
            reason: "Incident response".to_string(),
            requested_at: now,
            expires_at: now + Duration::seconds(expires_offset_secs),
            status,
        }
    }

    #[test]
    fn test_is_session_active_active_future() {
        let user_id = Uuid::new_v4();
        let session = make_session(user_id, SessionStatus::Active, 3600, None);
        assert!(is_session_active(&session));
    }

    #[test]
    fn test_is_session_active_expired_time() {
        let user_id = Uuid::new_v4();
        let session = make_session(user_id, SessionStatus::Active, -1, None);
        assert!(!is_session_active(&session));
    }

    #[test]
    fn test_is_session_active_wrong_status() {
        let user_id = Uuid::new_v4();
        let session = make_session(user_id, SessionStatus::PendingApproval, 3600, None);
        assert!(!is_session_active(&session));
    }

    #[test]
    fn test_is_request_valid_pending_future() {
        let req = make_request(RequestStatus::Pending, 3600);
        assert!(is_request_valid(&req));
    }

    #[test]
    fn test_pending_sessions_filter() {
        let user_id = Uuid::new_v4();
        let sessions = vec![
            make_session(user_id, SessionStatus::PendingApproval, 3600, None),
            make_session(user_id, SessionStatus::Active, 3600, None),
            make_session(user_id, SessionStatus::PendingApproval, 3600, None),
            make_session(user_id, SessionStatus::Terminated, -10, None),
        ];
        let pending = pending_sessions(&sessions);
        assert_eq!(pending.len(), 2);
        for s in &pending {
            assert_eq!(s.status, SessionStatus::PendingApproval);
        }
    }

    #[test]
    fn test_active_session_count() {
        let target_user = Uuid::new_v4();
        let other_user = Uuid::new_v4();
        let sessions = vec![
            make_session(target_user, SessionStatus::Active, 3600, None),
            make_session(target_user, SessionStatus::Active, 3600, None),
            make_session(target_user, SessionStatus::Terminated, -10, None),
            make_session(other_user, SessionStatus::Active, 3600, None),
        ];
        assert_eq!(active_session_count(&sessions, &target_user), 2);
        assert_eq!(active_session_count(&sessions, &other_user), 1);
    }

    #[test]
    fn test_is_recorded_with_path() {
        let user_id = Uuid::new_v4();
        let session = make_session(
            user_id,
            SessionStatus::Active,
            3600,
            Some("/recordings/session-001.cast".to_string()),
        );
        assert!(is_recorded(&session));
    }

    #[test]
    fn test_is_recorded_without_path() {
        let user_id = Uuid::new_v4();
        let session = make_session(user_id, SessionStatus::Active, 3600, None);
        assert!(!is_recorded(&session));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Session management — create, validate, refresh, invalidate.
//!
//! Sessions layer on top of OIDC tokens, providing server-side session state
//! for the CAVE platform (e.g., portal UI sessions, API sessions).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Default session TTL.
const DEFAULT_SESSION_TTL_MINUTES: i64 = 60;
/// Maximum session idle time before expiry.
const IDLE_TIMEOUT_MINUTES: i64 = 30;

/// A platform session record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub tenant_id: String,
    pub env: String,
    /// The OIDC access token bound to this session.
    pub access_token: Option<String>,
    /// The OIDC refresh token for renewing this session.
    pub refresh_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub revoked: bool,
    /// Session metadata (e.g., client version, device type).
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Session {
    pub fn new(user_id: Uuid, tenant_id: String, env: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id,
            tenant_id,
            env,
            access_token: None,
            refresh_token: None,
            created_at: now,
            expires_at: now + Duration::minutes(DEFAULT_SESSION_TTL_MINUTES),
            last_active_at: now,
            ip_address: None,
            user_agent: None,
            revoked: false,
            metadata: HashMap::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        if self.revoked {
            return false;
        }
        let now = Utc::now();
        if now > self.expires_at {
            return false;
        }
        // Idle timeout check
        let idle_for = now - self.last_active_at;
        if idle_for > Duration::minutes(IDLE_TIMEOUT_MINUTES) {
            return false;
        }
        true
    }

    /// Touch the session — resets idle timeout.
    pub fn touch(&mut self) {
        self.last_active_at = Utc::now();
    }
}

/// Request to create a new session.
#[derive(Debug, Clone)]
pub struct CreateSessionRequest {
    pub user_id: Uuid,
    pub tenant_id: String,
    pub env: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub ttl_minutes: Option<i64>,
}

/// Session manager — stores and manages sessions in memory.
#[derive(Clone)]
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<Uuid, Session>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create and store a new session.
    pub async fn create(&self, req: CreateSessionRequest) -> Session {
        let mut session = Session::new(req.user_id, req.tenant_id, req.env);
        session.access_token = req.access_token;
        session.refresh_token = req.refresh_token;
        session.ip_address = req.ip_address;
        session.user_agent = req.user_agent;
        if let Some(ttl) = req.ttl_minutes {
            session.expires_at = session.created_at + Duration::minutes(ttl);
        }
        self.sessions.write().await.insert(session.id, session.clone());
        session
    }

    /// Validate a session — returns active session or error.
    pub async fn validate(&self, id: Uuid) -> Result<Session, String> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&id)
            .ok_or_else(|| format!("Session {id} not found"))?;

        if !session.is_active() {
            return Err(format!("Session {id} is expired or revoked"));
        }

        Ok(session.clone())
    }

    /// Touch a session to reset its idle timer. Returns updated session.
    pub async fn touch(&self, id: Uuid) -> Result<Session, String> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| format!("Session {id} not found"))?;

        if !session.is_active() {
            return Err(format!("Session {id} is expired or revoked"));
        }

        session.touch();
        Ok(session.clone())
    }

    /// Update tokens on a session (after token refresh).
    pub async fn refresh_tokens(
        &self,
        id: Uuid,
        new_access_token: String,
        new_refresh_token: Option<String>,
    ) -> Result<Session, String> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| format!("Session {id} not found"))?;

        if !session.is_active() {
            return Err("Cannot refresh expired session".to_string());
        }

        session.access_token = Some(new_access_token);
        if let Some(rt) = new_refresh_token {
            session.refresh_token = Some(rt);
        }
        // Extend session TTL on refresh
        session.expires_at = Utc::now() + Duration::minutes(DEFAULT_SESSION_TTL_MINUTES);
        session.touch();
        Ok(session.clone())
    }

    /// Invalidate a single session.
    pub async fn invalidate(&self, id: Uuid) -> Result<(), String> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&id)
            .ok_or_else(|| format!("Session {id} not found"))?;
        session.revoked = true;
        Ok(())
    }

    /// Invalidate all sessions for a user (e.g., on password change).
    pub async fn invalidate_all(&self, user_id: Uuid) {
        let mut sessions = self.sessions.write().await;
        for session in sessions.values_mut() {
            if session.user_id == user_id {
                session.revoked = true;
            }
        }
    }

    /// List active sessions for a user.
    pub async fn list_user_sessions(&self, user_id: Uuid) -> Vec<Session> {
        self.sessions
            .read()
            .await
            .values()
            .filter(|s| s.user_id == user_id && s.is_active())
            .cloned()
            .collect()
    }

    /// Purge expired sessions (maintenance task).
    pub async fn purge_expired(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let before = sessions.len();
        sessions.retain(|_, s| !s.revoked && Utc::now() < s.expires_at);
        before - sessions.len()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(user_id: Uuid) -> CreateSessionRequest {
        CreateSessionRequest {
            user_id,
            tenant_id: "acme".to_string(),
            env: "prod".to_string(),
            access_token: Some("at-token".to_string()),
            refresh_token: Some("rt-token".to_string()),
            ip_address: Some("10.0.0.1".to_string()),
            user_agent: None,
            ttl_minutes: None,
        }
    }

    #[tokio::test]
    async fn session_create_and_validate() {
        let mgr = SessionManager::new();
        let user_id = Uuid::new_v4();
        let session = mgr.create(make_req(user_id)).await;
        let validated = mgr.validate(session.id).await.unwrap();
        assert_eq!(validated.user_id, user_id);
        assert!(validated.is_active());
    }

    #[tokio::test]
    async fn session_invalidate() {
        let mgr = SessionManager::new();
        let user_id = Uuid::new_v4();
        let session = mgr.create(make_req(user_id)).await;

        mgr.invalidate(session.id).await.unwrap();
        let err = mgr.validate(session.id).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn session_invalidate_all() {
        let mgr = SessionManager::new();
        let user_id = Uuid::new_v4();

        let s1 = mgr.create(make_req(user_id)).await;
        let s2 = mgr.create(make_req(user_id)).await;

        mgr.invalidate_all(user_id).await;

        assert!(mgr.validate(s1.id).await.is_err());
        assert!(mgr.validate(s2.id).await.is_err());
    }

    #[tokio::test]
    async fn session_refresh_tokens() {
        let mgr = SessionManager::new();
        let user_id = Uuid::new_v4();
        let session = mgr.create(make_req(user_id)).await;

        let updated = mgr
            .refresh_tokens(session.id, "new-at".to_string(), Some("new-rt".to_string()))
            .await
            .unwrap();

        assert_eq!(updated.access_token.as_deref(), Some("new-at"));
        assert_eq!(updated.refresh_token.as_deref(), Some("new-rt"));
    }

    #[tokio::test]
    async fn session_purge_expired() {
        let mgr = SessionManager::new();
        let user_id = Uuid::new_v4();

        // Create session with negative TTL (already expired)
        let mut req = make_req(user_id);
        req.ttl_minutes = Some(-1); // Expired
        mgr.create(req).await;

        // Create active session
        mgr.create(make_req(user_id)).await;

        let purged = mgr.purge_expired().await;
        assert_eq!(purged, 1);

        let remaining = mgr.list_user_sessions(user_id).await;
        assert_eq!(remaining.len(), 1);
    }

    #[tokio::test]
    async fn session_list_user_sessions() {
        let mgr = SessionManager::new();
        let u1 = Uuid::new_v4();
        let u2 = Uuid::new_v4();

        mgr.create(make_req(u1)).await;
        mgr.create(make_req(u1)).await;
        mgr.create(make_req(u2)).await;

        let u1_sessions = mgr.list_user_sessions(u1).await;
        assert_eq!(u1_sessions.len(), 2);

        let u2_sessions = mgr.list_user_sessions(u2).await;
        assert_eq!(u2_sessions.len(), 1);
    }
}

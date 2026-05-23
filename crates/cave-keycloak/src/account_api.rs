// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Account REST API — user self-service surface (read profile, change
//! password, list active sessions, register WebAuthn, enroll TOTP).
//!
//! Upstream: `services/src/main/java/org/keycloak/services/resources/account/AccountRestService.java`.

use serde::{Deserialize, Serialize};

use crate::models::User;
use crate::session::UserSession;

pub fn account_url(base: &str, realm_id: &str) -> String {
    format!("{}/realms/{}/account", base.trim_end_matches('/'), realm_id)
}

pub fn account_password_url(base: &str, realm_id: &str) -> String {
    format!("{}/credentials/password", account_url(base, realm_id))
}

pub fn account_totp_url(base: &str, realm_id: &str) -> String {
    format!("{}/credentials/totp", account_url(base, realm_id))
}

pub fn account_webauthn_url(base: &str, realm_id: &str) -> String {
    format!("{}/credentials/webauthn", account_url(base, realm_id))
}

pub fn account_sessions_url(base: &str, realm_id: &str) -> String {
    format!("{}/sessions", account_url(base, realm_id))
}

/// The minimal self-service profile a user sees on `GET /account`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub active_session_count: usize,
}

impl AccountProfile {
    pub fn from(user: &User, sessions: &[UserSession]) -> Self {
        Self {
            user_id: user.id.clone(),
            username: user.username.clone(),
            email: user.email.clone(),
            email_verified: user.email_verified,
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            active_session_count: sessions.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Realm;
    use crate::session::SessionStore;
    use chrono::Utc;
    use std::collections::BTreeMap;

    fn user() -> User {
        User {
            id: "u1".into(),
            realm_id: "r1".into(),
            username: "alice".into(),
            enabled: true,
            email: Some("a@x".into()),
            email_verified: true,
            first_name: Some("Alice".into()),
            last_name: Some("X".into()),
            federated_link: None,
            group_ids: vec![],
            realm_role_ids: vec![],
            client_role_ids: vec![],
            attributes: BTreeMap::new(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn account_url_builders_are_realm_scoped() {
        assert_eq!(account_url("https://iam", "r1"), "https://iam/realms/r1/account");
        assert!(account_password_url("https://iam", "r1").ends_with("/credentials/password"));
        assert!(account_totp_url("https://iam", "r1").ends_with("/credentials/totp"));
        assert!(account_webauthn_url("https://iam", "r1").ends_with("/credentials/webauthn"));
        assert!(account_sessions_url("https://iam", "r1").ends_with("/sessions"));
    }

    #[test]
    fn account_profile_includes_session_count() {
        let s = SessionStore::default();
        let r = Realm::new("r1", "t1", "R1");
        let u = user();
        s.create(&r, &u, "password", false, false);
        s.create(&r, &u, "password", true, false);
        let sessions = s.list_for_user("r1", "u1");
        let p = AccountProfile::from(&u, &sessions);
        assert_eq!(p.user_id, "u1");
        assert_eq!(p.active_session_count, 2);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SSO + offline user sessions, access token issuance, and OIDC IdToken
//! claims assembly.
//!
//! Upstream:
//!   * `services/src/main/java/org/keycloak/protocol/oidc/TokenManager.java`
//!   * `services/src/main/java/org/keycloak/services/managers/UserSessionManager.java`
//!   * `model/src/main/java/org/keycloak/models/UserSessionModel.java`

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::error::{KeycloakError, Result};
use crate::models::{Realm, User};
use crate::signer::SignerRegistry;

/// Live SSO session for a user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSession {
    pub id: String,
    pub realm_id: String,
    pub user_id: String,
    pub auth_method: String,
    pub remember_me: bool,
    pub started_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub idle_ttl: i64,
    pub max_ttl: i64,
    pub offline: bool,
    pub client_sessions: BTreeMap<String, ClientSession>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientSession {
    pub client_id: String,
    pub scope: String,
    pub state: Option<String>,
    pub started_at: DateTime<Utc>,
}

pub struct SessionStore {
    inner: Mutex<BTreeMap<String, UserSession>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self { inner: Mutex::new(BTreeMap::new()) }
    }
}

impl SessionStore {
    pub fn create(&self, realm: &Realm, user: &User, auth_method: &str, remember_me: bool, offline: bool) -> UserSession {
        let now = Utc::now();
        let ttl_max = if offline { realm.offline_session_idle_seconds as i64 * 4 } else { realm.sso_session_max_seconds as i64 };
        let s = UserSession {
            id: Uuid::new_v4().to_string(),
            realm_id: realm.id.clone(),
            user_id: user.id.clone(),
            auth_method: auth_method.to_string(),
            remember_me,
            started_at: now,
            last_seen_at: now,
            idle_ttl: realm.sso_session_idle_seconds as i64,
            max_ttl: ttl_max,
            offline,
            client_sessions: BTreeMap::new(),
        };
        let mut g = self.inner.lock().unwrap();
        g.insert(s.id.clone(), s.clone());
        s
    }

    pub fn touch(&self, id: &str) -> Result<UserSession> {
        let mut g = self.inner.lock().unwrap();
        let s = g.get_mut(id).ok_or_else(|| KeycloakError::InvalidGrant("unknown session".into()))?;
        let now = Utc::now();
        let idle = (now - s.last_seen_at).num_seconds();
        let age = (now - s.started_at).num_seconds();
        if idle > s.idle_ttl || age > s.max_ttl {
            g.remove(id);
            return Err(KeycloakError::TokenExpired);
        }
        s.last_seen_at = now;
        Ok(s.clone())
    }

    pub fn attach_client(&self, id: &str, cs: ClientSession) -> Result<UserSession> {
        let mut g = self.inner.lock().unwrap();
        let s = g.get_mut(id).ok_or_else(|| KeycloakError::InvalidGrant("unknown session".into()))?;
        s.client_sessions.insert(cs.client_id.clone(), cs);
        Ok(s.clone())
    }

    pub fn logout(&self, id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        g.remove(id).is_some()
    }

    pub fn list_for_user(&self, realm_id: &str, user_id: &str) -> Vec<UserSession> {
        let g = self.inner.lock().unwrap();
        g.values()
            .filter(|s| s.realm_id == realm_id && s.user_id == user_id)
            .cloned()
            .collect()
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

// ─── OIDC token assembly ────────────────────────────────────────────────

/// Inputs to assemble an Access Token / IdToken pair.
pub struct TokenClaims<'a> {
    pub realm: &'a Realm,
    pub user: &'a User,
    pub client_id: &'a str,
    pub session_id: &'a str,
    pub scope: &'a str,
    pub effective_roles: &'a [String],
    pub nonce: Option<&'a str>,
    pub issuer_url: &'a str,
}

/// Issue an access token + an OIDC IdToken. Both are compact JWS strings.
/// The cave-keycloak issuer URL convention is
/// `https://iam.cave.svc/realms/{realm_id}`.
pub fn issue_tokens(claims: TokenClaims<'_>, signer: &SignerRegistry, alg: &str, kid: &str) -> Result<IssuedTokens> {
    let now = Utc::now();
    let exp = now + Duration::seconds(claims.realm.access_token_lifespan_seconds as i64);
    let header = serde_json::json!({ "alg": alg, "kid": kid, "typ": "JWT" });
    let access = serde_json::json!({
        "iss": claims.issuer_url,
        "sub": claims.user.id,
        "aud": claims.client_id,
        "exp": exp.timestamp(),
        "iat": now.timestamp(),
        "scope": claims.scope,
        "session_state": claims.session_id,
        "realm_access": { "roles": claims.effective_roles },
        "preferred_username": claims.user.username,
        "email": claims.user.email,
        "email_verified": claims.user.email_verified,
        "typ": "Bearer",
    });
    let access_jws = signer.sign_compact(&claims.realm.id, &header, &access)?;
    let mut id = serde_json::json!({
        "iss": claims.issuer_url,
        "sub": claims.user.id,
        "aud": claims.client_id,
        "exp": exp.timestamp(),
        "iat": now.timestamp(),
        "auth_time": now.timestamp(),
        "session_state": claims.session_id,
        "preferred_username": claims.user.username,
        "email": claims.user.email,
        "email_verified": claims.user.email_verified,
        "typ": "ID",
    });
    if let Some(n) = claims.nonce {
        id["nonce"] = serde_json::Value::String(n.to_string());
    }
    let id_jws = signer.sign_compact(&claims.realm.id, &header, &id)?;
    Ok(IssuedTokens {
        access_token: access_jws,
        id_token: id_jws,
        expires_in: claims.realm.access_token_lifespan_seconds,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedTokens {
    pub access_token: String,
    pub id_token: String,
    pub expires_in: u32,
}

/// Userinfo response — subset of the OIDC standard claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInfo {
    pub sub: String,
    pub preferred_username: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}

pub fn userinfo_from(user: &User) -> UserInfo {
    let name = match (user.first_name.as_deref(), user.last_name.as_deref()) {
        (Some(f), Some(l)) => Some(format!("{} {}", f, l)),
        (Some(f), None) => Some(f.to_string()),
        (None, Some(l)) => Some(l.to_string()),
        _ => None,
    };
    UserInfo {
        sub: user.id.clone(),
        preferred_username: user.username.clone(),
        email: user.email.clone(),
        email_verified: user.email_verified,
        name,
        given_name: user.first_name.clone(),
        family_name: user.last_name.clone(),
    }
}

/// PKCE / nonce / state utility — random URL-safe base64 string.
pub fn random_token(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::SigningKeyEntry;
    use std::collections::BTreeMap;

    fn realm() -> Realm {
        Realm::new("r1", "t1", "R1")
    }

    fn user() -> User {
        User {
            id: "u1".into(),
            realm_id: "r1".into(),
            username: "alice".into(),
            enabled: true,
            email: Some("alice@x".into()),
            email_verified: true,
            first_name: Some("Alice".into()),
            last_name: Some("X".into()),
            federated_link: None,
            group_ids: vec![],
            realm_role_ids: vec!["admin".into()],
            client_role_ids: vec![],
            attributes: BTreeMap::new(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn session_create_and_touch() {
        let s = SessionStore::default();
        let sess = s.create(&realm(), &user(), "password", false, false);
        let back = s.touch(&sess.id).unwrap();
        assert_eq!(back.id, sess.id);
        assert!(back.last_seen_at >= back.started_at);
    }

    #[test]
    fn session_offline_uses_offline_idle_ttl_bound() {
        let mut r = realm();
        r.offline_session_idle_seconds = 100;
        let s = SessionStore::default();
        let sess = s.create(&r, &user(), "password", false, true);
        assert!(sess.offline);
        assert!(sess.max_ttl >= 100);
    }

    #[test]
    fn session_logout_removes_entry() {
        let s = SessionStore::default();
        let sess = s.create(&realm(), &user(), "password", false, false);
        assert!(s.logout(&sess.id));
        assert!(!s.logout(&sess.id));
    }

    #[test]
    fn list_for_user_filters_by_user_and_realm() {
        let s = SessionStore::default();
        let r = realm();
        let mut u2 = user();
        u2.id = "u2".into();
        s.create(&r, &user(), "password", false, false);
        s.create(&r, &u2, "password", false, false);
        let list = s.list_for_user("r1", "u1");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn issue_tokens_signs_access_and_id() {
        let reg = SignerRegistry::default();
        reg.install(
            "r1",
            SigningKeyEntry::es256_from_seed("kid-1", &[7u8; 32]).unwrap(),
            true,
        );
        let r = realm();
        let u = user();
        let s = SessionStore::default();
        let sess = s.create(&r, &u, "password", false, false);
        let tk = issue_tokens(
            TokenClaims {
                realm: &r,
                user: &u,
                client_id: "spa",
                session_id: &sess.id,
                scope: "openid profile email",
                effective_roles: &["admin".to_string()],
                nonce: Some("n-1"),
                issuer_url: "https://iam.cave.svc/realms/r1",
            },
            &reg,
            "ES256",
            "kid-1",
        )
        .unwrap();
        assert!(tk.access_token.contains('.'));
        assert!(tk.id_token.contains('.'));
        let (_h, payload) = reg.verify_compact("r1", &tk.access_token).unwrap();
        assert_eq!(payload["sub"], "u1");
        assert_eq!(payload["aud"], "spa");
    }

    #[test]
    fn userinfo_assembles_name_from_first_last() {
        let info = userinfo_from(&user());
        assert_eq!(info.preferred_username, "alice");
        assert_eq!(info.name.as_deref(), Some("Alice X"));
    }

    #[test]
    fn random_token_has_expected_length() {
        let t = random_token(24);
        assert!(t.len() >= 32);
    }
}

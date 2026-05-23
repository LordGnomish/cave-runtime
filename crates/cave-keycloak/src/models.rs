// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core data types — Realm, User, Group, Role, Client, Credential, Session.
//!
//! Upstream mapping:
//!   * `model/src/main/java/org/keycloak/representations/idm/RealmRepresentation.java`
//!   * `model/src/main/java/org/keycloak/representations/idm/UserRepresentation.java`
//!   * `model/src/main/java/org/keycloak/representations/idm/ClientRepresentation.java`
//!   * `model/src/main/java/org/keycloak/representations/idm/RoleRepresentation.java`
//!   * `model/src/main/java/org/keycloak/representations/idm/GroupRepresentation.java`
//!   * `model/src/main/java/org/keycloak/representations/idm/CredentialRepresentation.java`

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{KeycloakError, Result};

/// A Keycloak realm. Identity boundary — every user, role, client, session
/// is scoped to exactly one realm, and every realm is scoped to exactly one
/// `tenant_id` (cave invariant — no Keycloak equivalent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Realm {
    pub id: String,
    pub tenant_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub access_token_lifespan_seconds: u32,
    pub sso_session_idle_seconds: u32,
    pub sso_session_max_seconds: u32,
    pub offline_session_idle_seconds: u32,
    pub refresh_token_max_reuse: u8,
    pub registration_allowed: bool,
    pub login_with_email_allowed: bool,
    pub duplicate_emails_allowed: bool,
    pub brute_force_protected: bool,
    pub password_policy: PasswordPolicy,
    pub attributes: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
}

impl Realm {
    pub fn new(id: impl Into<String>, tenant_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        let id = id.into();
        let dn = display_name.into();
        let display_name = if dn.is_empty() { id.clone() } else { dn };
        Self {
            display_name,
            id,
            tenant_id: tenant_id.into(),
            enabled: true,
            access_token_lifespan_seconds: 300,
            sso_session_idle_seconds: 1800,
            sso_session_max_seconds: 36_000,
            offline_session_idle_seconds: 2_592_000,
            refresh_token_max_reuse: 0,
            registration_allowed: false,
            login_with_email_allowed: true,
            duplicate_emails_allowed: false,
            brute_force_protected: true,
            password_policy: PasswordPolicy::default(),
            attributes: BTreeMap::new(),
            created_at: Utc::now(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(KeycloakError::InvalidRequest("realm.id empty".into()));
        }
        if self.tenant_id.is_empty() {
            return Err(KeycloakError::InvalidRequest("realm.tenant_id empty".into()));
        }
        if self.access_token_lifespan_seconds == 0 {
            return Err(KeycloakError::InvalidRequest("access_token_lifespan_seconds == 0".into()));
        }
        if self.sso_session_idle_seconds > self.sso_session_max_seconds {
            return Err(KeycloakError::InvalidRequest("sso_session_idle > sso_session_max".into()));
        }
        Ok(())
    }
}

/// Password policy — RealmRepresentation.passwordPolicy parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordPolicy {
    pub min_length: u8,
    pub require_uppercase: u8,
    pub require_lowercase: u8,
    pub require_digit: u8,
    pub require_special: u8,
    pub history_count: u8,
    pub hash_algorithm: HashAlgorithm,
    pub hash_iterations: u32,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            min_length: 8,
            require_uppercase: 0,
            require_lowercase: 0,
            require_digit: 0,
            require_special: 0,
            history_count: 0,
            hash_algorithm: HashAlgorithm::Pbkdf2Sha512,
            hash_iterations: 210_000,
        }
    }
}

/// Hashing algorithm — Keycloak CredentialModel.algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    Pbkdf2Sha256,
    Pbkdf2Sha512,
    Argon2,
}

/// User account — `UserRepresentation` parity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub realm_id: String,
    pub username: String,
    pub enabled: bool,
    pub email: Option<String>,
    pub email_verified: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub federated_link: Option<FederatedIdentity>,
    pub group_ids: Vec<String>,
    pub realm_role_ids: Vec<String>,
    pub client_role_ids: Vec<(String, String)>, // (client_id, role_id)
    pub attributes: BTreeMap<String, Vec<String>>,
    pub created_at: DateTime<Utc>,
}

impl User {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() || self.realm_id.is_empty() {
            return Err(KeycloakError::InvalidRequest("user id/realm_id empty".into()));
        }
        if self.username.is_empty() {
            return Err(KeycloakError::InvalidRequest("username empty".into()));
        }
        if self.username.chars().any(char::is_whitespace) {
            return Err(KeycloakError::InvalidRequest("username contains whitespace".into()));
        }
        if let Some(e) = &self.email {
            if !e.contains('@') || e.starts_with('@') || e.ends_with('@') {
                return Err(KeycloakError::InvalidRequest("email format invalid".into()));
            }
        }
        Ok(())
    }
}

/// A link to an external IDP (Google/GitHub/SAML peer / LDAP entry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedIdentity {
    pub provider_alias: String,
    pub provider_user_id: String,
    pub provider_username: String,
}

/// Group — `GroupRepresentation`. Hierarchy via `parent_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub realm_id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub attributes: BTreeMap<String, Vec<String>>,
    pub realm_role_ids: Vec<String>,
}

/// Role — `RoleRepresentation`. `client_id == None` means realm-level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: String,
    pub realm_id: String,
    pub client_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    /// Composite role expansion — IDs of roles this role implies.
    pub composite_ids: Vec<String>,
}

/// Client — `ClientRepresentation`. OAuth2 client / OIDC RP / SAML SP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    pub realm_id: String,
    pub client_id: String,
    pub name: String,
    pub enabled: bool,
    pub protocol: Protocol,
    pub public_client: bool,
    pub client_secret_hash: Option<String>,
    pub redirect_uris: Vec<String>,
    pub web_origins: Vec<String>,
    pub default_scopes: Vec<String>,
    pub optional_scopes: Vec<String>,
    pub allowed_grant_types: Vec<GrantType>,
    pub require_pkce: bool,
    pub access_token_lifespan_seconds: Option<u32>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    OpenIdConnect,
    Saml,
    Docker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum GrantType {
    AuthorizationCode,
    ClientCredentials,
    RefreshToken,
    DeviceCode,
    Password, // ROPC — deprecated path; allowed only when explicitly enabled
    TokenExchange,
}

impl Client {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() || self.realm_id.is_empty() || self.client_id.is_empty() {
            return Err(KeycloakError::InvalidRequest("client id/realm/client_id empty".into()));
        }
        for u in &self.redirect_uris {
            if u.is_empty() || u.contains(' ') || u.contains('\n') {
                return Err(KeycloakError::InvalidRequest(format!("redirect_uri invalid: {}", u)));
            }
        }
        if !self.public_client && self.client_secret_hash.is_none() {
            return Err(KeycloakError::InvalidRequest("confidential client requires client_secret_hash".into()));
        }
        Ok(())
    }

    /// RFC 6749 §3.1.2.1 — exact match of redirect URI (no wildcards in MVP).
    pub fn accepts_redirect_uri(&self, uri: &str) -> bool {
        self.redirect_uris.iter().any(|u| u == uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_new_defaults_are_sensible() {
        let r = Realm::new("master", "tenant-1", "Master");
        assert_eq!(r.id, "master");
        assert_eq!(r.tenant_id, "tenant-1");
        assert!(r.enabled);
        assert_eq!(r.access_token_lifespan_seconds, 300);
        assert!(r.brute_force_protected);
        r.validate().unwrap();
    }

    #[test]
    fn realm_validate_rejects_empty_id() {
        let mut r = Realm::new("x", "t", "X");
        r.id = String::new();
        assert!(r.validate().is_err());
    }

    #[test]
    fn realm_validate_rejects_idle_above_max() {
        let mut r = Realm::new("x", "t", "X");
        r.sso_session_idle_seconds = 999_999;
        r.sso_session_max_seconds = 1;
        assert!(r.validate().is_err());
    }

    #[test]
    fn user_validate_rejects_whitespace_username() {
        let u = User {
            id: "u1".into(),
            realm_id: "r1".into(),
            username: "alice bob".into(),
            enabled: true,
            email: None,
            email_verified: false,
            first_name: None,
            last_name: None,
            federated_link: None,
            group_ids: vec![],
            realm_role_ids: vec![],
            client_role_ids: vec![],
            attributes: BTreeMap::new(),
            created_at: Utc::now(),
        };
        assert!(u.validate().is_err());
    }

    #[test]
    fn user_validate_rejects_bad_email() {
        let u = User {
            id: "u1".into(),
            realm_id: "r1".into(),
            username: "alice".into(),
            enabled: true,
            email: Some("no-at-sign".into()),
            email_verified: false,
            first_name: None,
            last_name: None,
            federated_link: None,
            group_ids: vec![],
            realm_role_ids: vec![],
            client_role_ids: vec![],
            attributes: BTreeMap::new(),
            created_at: Utc::now(),
        };
        assert!(u.validate().is_err());
    }

    #[test]
    fn client_redirect_uri_is_exact_match() {
        let c = Client {
            id: "c1".into(),
            realm_id: "r1".into(),
            client_id: "test".into(),
            name: "Test".into(),
            enabled: true,
            protocol: Protocol::OpenIdConnect,
            public_client: true,
            client_secret_hash: None,
            redirect_uris: vec!["https://example.com/cb".into()],
            web_origins: vec![],
            default_scopes: vec!["openid".into()],
            optional_scopes: vec![],
            allowed_grant_types: vec![GrantType::AuthorizationCode],
            require_pkce: true,
            access_token_lifespan_seconds: None,
            attributes: BTreeMap::new(),
        };
        c.validate().unwrap();
        assert!(c.accepts_redirect_uri("https://example.com/cb"));
        assert!(!c.accepts_redirect_uri("https://example.com/cb?x=1"));
    }

    #[test]
    fn client_confidential_requires_secret_hash() {
        let c = Client {
            id: "c1".into(),
            realm_id: "r1".into(),
            client_id: "test".into(),
            name: "Test".into(),
            enabled: true,
            protocol: Protocol::OpenIdConnect,
            public_client: false,
            client_secret_hash: None,
            redirect_uris: vec!["https://x/cb".into()],
            web_origins: vec![],
            default_scopes: vec![],
            optional_scopes: vec![],
            allowed_grant_types: vec![GrantType::ClientCredentials],
            require_pkce: false,
            access_token_lifespan_seconds: None,
            attributes: BTreeMap::new(),
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn password_policy_default_is_pbkdf2_sha512() {
        let p = PasswordPolicy::default();
        assert_eq!(p.hash_algorithm, HashAlgorithm::Pbkdf2Sha512);
        assert!(p.hash_iterations >= 100_000);
    }
}

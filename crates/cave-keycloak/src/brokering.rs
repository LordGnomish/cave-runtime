// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Identity brokering — external OIDC IDPs (Google / GitHub / Microsoft /
//! generic) plus the just-in-time provisioning helper that pulls the
//! external `sub` into a federated cave-keycloak user.
//!
//! Upstream:
//!   * `services/src/main/java/org/keycloak/broker/oidc/AbstractOAuth2IdentityProvider.java`
//!   * `services/src/main/java/org/keycloak/broker/oidc/OIDCIdentityProvider.java`
//!   * `services/src/main/java/org/keycloak/broker/oidc/IdentityProviderConfig.java`

use serde::{Deserialize, Serialize};

use crate::error::{KeycloakError, Result};
use crate::models::{FederatedIdentity, User};

/// External IDP family. The cave port lays out the well-known endpoints
/// so the operator can wire a brokered login without filling in 8
/// endpoint URLs by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrokerFamily {
    Google,
    GitHub,
    MicrosoftOidc,
    OidcGeneric,
}

/// Configured external IDP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalIdp {
    pub alias: String,
    pub realm_id: String,
    pub family: BrokerFamily,
    pub client_id: String,
    pub client_secret_keychain_handle: String,
    pub authorization_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub jwks_url: Option<String>,
    pub default_scopes: Vec<String>,
    pub trust_email: bool,
}

impl ExternalIdp {
    pub fn validate(&self) -> Result<()> {
        if !self.client_secret_keychain_handle.starts_with("keychain:") {
            return Err(KeycloakError::BrokeringError(
                "client_secret must reference keychain handle".into(),
            ));
        }
        if self.alias.is_empty() {
            return Err(KeycloakError::BrokeringError("alias empty".into()));
        }
        if self.authorization_url.is_empty() || self.token_url.is_empty() {
            return Err(KeycloakError::BrokeringError("authorization_url / token_url empty".into()));
        }
        Ok(())
    }

    /// Apply the family default endpoints for the common cases so the
    /// operator only fills in `client_id` + keychain handle.
    pub fn with_family_defaults(mut self) -> Self {
        match self.family {
            BrokerFamily::Google => {
                self.authorization_url = "https://accounts.google.com/o/oauth2/v2/auth".into();
                self.token_url = "https://oauth2.googleapis.com/token".into();
                self.userinfo_url = "https://openidconnect.googleapis.com/v1/userinfo".into();
                self.jwks_url = Some("https://www.googleapis.com/oauth2/v3/certs".into());
                self.default_scopes = vec!["openid".into(), "profile".into(), "email".into()];
                self.trust_email = true;
            }
            BrokerFamily::GitHub => {
                self.authorization_url = "https://github.com/login/oauth/authorize".into();
                self.token_url = "https://github.com/login/oauth/access_token".into();
                self.userinfo_url = "https://api.github.com/user".into();
                self.jwks_url = None;
                self.default_scopes = vec!["read:user".into(), "user:email".into()];
                self.trust_email = false;
            }
            BrokerFamily::MicrosoftOidc => {
                self.authorization_url = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into();
                self.token_url = "https://login.microsoftonline.com/common/oauth2/v2.0/token".into();
                self.userinfo_url = "https://graph.microsoft.com/oidc/userinfo".into();
                self.jwks_url = Some("https://login.microsoftonline.com/common/discovery/v2.0/keys".into());
                self.default_scopes = vec!["openid".into(), "profile".into(), "email".into()];
                self.trust_email = true;
            }
            BrokerFamily::OidcGeneric => {}
        }
        self
    }

    /// Build the `/authorize?…` URL the user-agent is redirected to.
    pub fn build_authorize_url(&self, redirect_uri: &str, state: &str, nonce: &str) -> String {
        let scope = self.default_scopes.join(" ");
        format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}",
            self.authorization_url,
            urlenc(&self.client_id),
            urlenc(redirect_uri),
            urlenc(&scope),
            urlenc(state),
            urlenc(nonce),
        )
    }
}

/// External claims returned by the IDP token + userinfo round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokeredIdentity {
    pub provider_alias: String,
    pub provider_user_id: String,
    pub provider_username: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

/// Just-in-time provisioning: map a brokered identity to a `User` shape.
/// The caller decides whether to insert (JIT) or look up (link-existing).
pub fn map_to_user(realm_id: &str, brokered: &BrokeredIdentity) -> User {
    use chrono::Utc;
    use std::collections::BTreeMap;
    let username = brokered.provider_username.clone();
    let id = format!("{}:{}", brokered.provider_alias, brokered.provider_user_id);
    User {
        id,
        realm_id: realm_id.into(),
        username,
        enabled: true,
        email: brokered.email.clone(),
        email_verified: brokered.email_verified,
        first_name: brokered.first_name.clone(),
        last_name: brokered.last_name.clone(),
        federated_link: Some(FederatedIdentity {
            provider_alias: brokered.provider_alias.clone(),
            provider_user_id: brokered.provider_user_id.clone(),
            provider_username: brokered.provider_username.clone(),
        }),
        group_ids: vec![],
        realm_role_ids: vec![],
        client_role_ids: vec![],
        attributes: BTreeMap::new(),
        created_at: Utc::now(),
    }
}

fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google() -> ExternalIdp {
        ExternalIdp {
            alias: "google".into(),
            realm_id: "r1".into(),
            family: BrokerFamily::Google,
            client_id: "cave-google-client-id".into(),
            client_secret_keychain_handle: "keychain:cave-keycloak/idp/google".into(),
            authorization_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            jwks_url: None,
            default_scopes: vec![],
            trust_email: false,
        }
        .with_family_defaults()
    }

    #[test]
    fn google_defaults_fill_in_endpoints() {
        let g = google();
        assert!(g.authorization_url.contains("accounts.google.com"));
        assert!(g.token_url.contains("oauth2.googleapis.com"));
        assert!(g.default_scopes.contains(&"email".to_string()));
        assert!(g.trust_email);
    }

    #[test]
    fn github_defaults_do_not_trust_email() {
        let g = ExternalIdp {
            alias: "gh".into(),
            realm_id: "r1".into(),
            family: BrokerFamily::GitHub,
            client_id: "cid".into(),
            client_secret_keychain_handle: "keychain:cave-keycloak/idp/gh".into(),
            authorization_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            jwks_url: None,
            default_scopes: vec![],
            trust_email: true,
        }
        .with_family_defaults();
        assert!(!g.trust_email);
        assert!(g.authorization_url.contains("github.com"));
    }

    #[test]
    fn microsoft_defaults_set_jwks_url() {
        let m = ExternalIdp {
            alias: "ms".into(),
            realm_id: "r1".into(),
            family: BrokerFamily::MicrosoftOidc,
            client_id: "cid".into(),
            client_secret_keychain_handle: "keychain:cave-keycloak/idp/ms".into(),
            authorization_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            jwks_url: None,
            default_scopes: vec![],
            trust_email: false,
        }
        .with_family_defaults();
        assert!(m.jwks_url.unwrap().contains("microsoftonline"));
    }

    #[test]
    fn build_authorize_url_url_encodes_state() {
        let g = google();
        let url = g.build_authorize_url("https://cave/cb", "with space", "n-1");
        assert!(url.contains("state=with%20space"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fcave%2Fcb"));
    }

    #[test]
    fn validate_rejects_inline_client_secret() {
        let mut g = google();
        g.client_secret_keychain_handle = "literal".into();
        assert!(g.validate().is_err());
    }

    #[test]
    fn map_to_user_carries_federated_link() {
        let b = BrokeredIdentity {
            provider_alias: "google".into(),
            provider_user_id: "1234567890".into(),
            provider_username: "alice@example.com".into(),
            email: Some("alice@example.com".into()),
            email_verified: true,
            first_name: Some("Alice".into()),
            last_name: None,
        };
        let u = map_to_user("r1", &b);
        assert_eq!(u.id, "google:1234567890");
        assert!(u.federated_link.is_some());
        assert_eq!(u.federated_link.unwrap().provider_user_id, "1234567890");
    }
}

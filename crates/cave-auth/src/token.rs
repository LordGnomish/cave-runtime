// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Token management — introspection, revocation, ID token handling.
//! Wraps the Okta/Keycloak token endpoints and maintains a local revocation cache.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Token introspection response (RFC 7662).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionResponse {
    /// Whether the token is active.
    pub active: bool,
    pub scope: Option<String>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub token_type: Option<String>,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub sub: Option<String>,
    pub aud: Option<serde_json::Value>,
    pub iss: Option<String>,
    pub jti: Option<String>,
}

/// Token type for revocation (RFC 7009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenTypeHint {
    AccessToken,
    RefreshToken,
}

impl TokenTypeHint {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AccessToken => "access_token",
            Self::RefreshToken => "refresh_token",
        }
    }
}

/// Token manager — introspection + revocation with a local denylist.
pub struct TokenManager {
    introspection_endpoint: String,
    revocation_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
    /// JTI denylist — tokens revoked in this instance.
    denylist: Arc<RwLock<HashSet<String>>>,
    http: reqwest::Client,
}

impl TokenManager {
    pub fn new(
        introspection_endpoint: String,
        revocation_endpoint: String,
        client_id: String,
        client_secret: Option<String>,
    ) -> Self {
        Self {
            introspection_endpoint,
            revocation_endpoint,
            client_id,
            client_secret,
            denylist: Arc::new(RwLock::new(HashSet::new())),
            http: reqwest::Client::new(),
        }
    }

    /// Check if a JTI is locally denied (revoked in this instance).
    pub async fn is_denied(&self, jti: &str) -> bool {
        self.denylist.read().await.contains(jti)
    }

    /// Add a JTI to the local denylist (after revoking with IdP).
    pub async fn deny(&self, jti: String) {
        self.denylist.write().await.insert(jti);
    }

    /// Introspect a token via the IdP endpoint (RFC 7662).
    pub async fn introspect(&self, token: &str) -> Result<IntrospectionResponse, String> {
        let mut form = vec![
            ("token", token.to_string()),
            ("client_id", self.client_id.clone()),
        ];
        if let Some(ref secret) = self.client_secret {
            form.push(("client_secret", secret.clone()));
        }

        let resp = self
            .http
            .post(&self.introspection_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("Introspection request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Introspection endpoint returned {status}: {body}"));
        }

        let introspection: IntrospectionResponse = resp
            .json()
            .await
            .map_err(|e| format!("Introspection parse error: {e}"))?;

        // Also check local denylist
        if let Some(ref jti) = introspection.jti {
            if self.is_denied(jti).await {
                debug!(jti = %jti, "Token in local denylist");
                return Ok(IntrospectionResponse {
                    active: false,
                    ..introspection
                });
            }
        }

        Ok(introspection)
    }

    /// Revoke a token via the IdP endpoint (RFC 7009).
    pub async fn revoke(
        &self,
        token: &str,
        token_type_hint: TokenTypeHint,
    ) -> Result<(), String> {
        let mut form = vec![
            ("token", token.to_string()),
            ("token_type_hint", token_type_hint.as_str().to_string()),
            ("client_id", self.client_id.clone()),
        ];
        if let Some(ref secret) = self.client_secret {
            form.push(("client_secret", secret.clone()));
        }

        let resp = self
            .http
            .post(&self.revocation_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("Revocation request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, "Token revocation returned error: {body}");
            return Err(format!("Revocation endpoint returned {status}"));
        }

        Ok(())
    }

    /// Revoke a token and add to local denylist.
    pub async fn revoke_and_deny(
        &self,
        token: &str,
        jti: Option<String>,
        token_type_hint: TokenTypeHint,
    ) -> Result<(), String> {
        self.revoke(token, token_type_hint).await?;
        if let Some(jti) = jti {
            self.deny(jti).await;
        }
        Ok(())
    }
}

/// Parsed ID token claims (subset of OIDC standard claims).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdTokenClaims {
    pub sub: String,
    pub iss: String,
    pub aud: serde_json::Value,
    pub exp: i64,
    pub iat: i64,
    pub nonce: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub preferred_username: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub picture: Option<String>,
}

impl IdTokenClaims {
    pub fn is_expired(&self) -> bool {
        let exp = DateTime::from_timestamp(self.exp, 0).unwrap_or(DateTime::<Utc>::MIN_UTC);
        Utc::now() > exp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn denylist_add_and_check() {
        let mgr = TokenManager::new(
            "https://example.okta.com/oauth2/v1/introspect".to_string(),
            "https://example.okta.com/oauth2/v1/revoke".to_string(),
            "cave-client".to_string(),
            None,
        );
        assert!(!mgr.is_denied("jti-abc").await);
        mgr.deny("jti-abc".to_string()).await;
        assert!(mgr.is_denied("jti-abc").await);
    }

    #[tokio::test]
    async fn denylist_multiple_entries() {
        let mgr = TokenManager::new(
            "https://example.okta.com/oauth2/v1/introspect".to_string(),
            "https://example.okta.com/oauth2/v1/revoke".to_string(),
            "cave-client".to_string(),
            None,
        );
        mgr.deny("jti-1".to_string()).await;
        mgr.deny("jti-2".to_string()).await;
        assert!(mgr.is_denied("jti-1").await);
        assert!(mgr.is_denied("jti-2").await);
        assert!(!mgr.is_denied("jti-3").await);
    }

    #[test]
    fn id_token_expiry_check() {
        let past_claim = IdTokenClaims {
            sub: "sub".to_string(),
            iss: "iss".to_string(),
            aud: serde_json::Value::String("aud".to_string()),
            exp: 1000, // Unix epoch far in the past
            iat: 999,
            nonce: None,
            email: None,
            email_verified: None,
            name: None,
            preferred_username: None,
            given_name: None,
            family_name: None,
            picture: None,
        };
        assert!(past_claim.is_expired());
    }

    #[test]
    fn id_token_not_expired() {
        let future_claim = IdTokenClaims {
            sub: "sub".to_string(),
            iss: "iss".to_string(),
            aud: serde_json::Value::String("aud".to_string()),
            exp: 9_999_999_999, // Far future
            iat: 1_000_000_000,
            nonce: None,
            email: None,
            email_verified: None,
            name: None,
            preferred_username: None,
            given_name: None,
            family_name: None,
            picture: None,
        };
        assert!(!future_claim.is_expired());
    }

    #[test]
    fn token_type_hint_serialization() {
        assert_eq!(TokenTypeHint::AccessToken.as_str(), "access_token");
        assert_eq!(TokenTypeHint::RefreshToken.as_str(), "refresh_token");
    }
}

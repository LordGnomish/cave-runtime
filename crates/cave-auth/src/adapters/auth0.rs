//! Auth0 by Okta adapter.
//!
//! Validates JWTs via Auth0's JWKS endpoint (local validation — no round-trip
//! per request). Optionally calls the Management API for role membership.
//!
//! # Configuration
//!
//! ```toml
//! [auth]
//! backend       = "auth0"
//! auth0_domain  = "https://company.us.auth0.com"
//! auth0_audience = "https://api.example.com"
//! auth0_client_id     = "..."
//! auth0_client_secret = "..."
//! ```

use async_trait::async_trait;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::jwks::JwksCache;
use crate::provider::{AuthBackend, AuthBackendError, AuthBackendResult, VerifiedIdentity};

#[derive(Debug, Clone, Deserialize)]
pub struct Auth0Config {
    /// Auth0 domain, e.g. `https://company.us.auth0.com`
    pub domain: String,
    /// Expected audience (API identifier).
    pub audience: String,
    /// Management API client ID.
    pub client_id: String,
    /// Management API client secret.
    pub client_secret: String,
}

impl Auth0Config {
    pub fn jwks_uri(&self) -> String {
        format!("{}/.well-known/jwks.json", self.domain)
    }

    pub fn token_url(&self) -> String {
        format!("{}/oauth/token", self.domain)
    }

    pub fn mgmt_api_base(&self) -> String {
        format!("{}/api/v2", self.domain)
    }
}

/// Auth0 raw JWT payload.
#[derive(Debug, Deserialize)]
struct Auth0Claims {
    sub: String,
    email: Option<String>,
    /// Custom namespace for roles — Auth0 convention is to use a URL prefix.
    #[serde(rename = "https://cave.io/roles")]
    cave_roles: Option<Vec<String>>,
    #[serde(rename = "https://cave.io/groups")]
    cave_groups: Option<Vec<String>>,
    #[serde(rename = "https://cave.io/cave_uid")]
    cave_uid: Option<String>,
    #[serde(rename = "https://cave.io/tenant_id")]
    tenant_id: Option<String>,
    exp: i64,
}

/// Management API token response.
#[derive(Debug, Deserialize)]
struct MgmtTokenResponse {
    access_token: String,
}

/// Auth0 role object.
#[derive(Debug, Deserialize)]
struct Auth0Role {
    name: String,
}

/// Auth0 by Okta adapter.
pub struct Auth0Adapter {
    config: Auth0Config,
    jwks: Arc<JwksCache>,
    client: reqwest::Client,
}

impl Auth0Adapter {
    pub fn new(config: Auth0Config) -> Self {
        let jwks = Arc::new(JwksCache::new(config.jwks_uri()));
        Self { config, jwks, client: reqwest::Client::new() }
    }

    /// Acquire a Management API token via client_credentials.
    async fn acquire_mgmt_token(&self) -> AuthBackendResult<String> {
        let resp = self
            .client
            .post(self.config.token_url())
            .json(&serde_json::json!({
                "grant_type": "client_credentials",
                "client_id": self.config.client_id,
                "client_secret": self.config.client_secret,
                "audience": format!("{}/", self.config.mgmt_api_base()),
            }))
            .send()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Auth0 mgmt token request failed: {e}")))?;

        let tr: MgmtTokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Auth0 mgmt token parse failed: {e}")))?;

        Ok(tr.access_token)
    }

    async fn decode_jwt(&self, token: &str) -> AuthBackendResult<Auth0Claims> {
        let jwks = self
            .jwks
            .get_keys()
            .await
            .map_err(AuthBackendError::ProviderError)?;

        let mut last_err = String::new();
        for jwk in &jwks.keys {
            let Ok(decoding_key) = DecodingKey::from_jwk(jwk) else { continue };

            let alg = match jwk.common.key_algorithm {
                Some(jsonwebtoken::jwk::KeyAlgorithm::RS256) => Algorithm::RS256,
                Some(jsonwebtoken::jwk::KeyAlgorithm::RS384) => Algorithm::RS384,
                Some(jsonwebtoken::jwk::KeyAlgorithm::RS512) => Algorithm::RS512,
                _ => Algorithm::RS256,
            };

            let mut validation = Validation::new(alg);
            validation.set_audience(&[&self.config.audience]);
            validation.set_issuer(&[&format!("{}/", self.config.domain)]);

            match decode::<Auth0Claims>(token, &decoding_key, &validation) {
                Ok(data) => return Ok(data.claims),
                Err(e) => {
                    last_err = e.to_string();
                    continue;
                }
            }
        }

        Err(AuthBackendError::InvalidToken(last_err))
    }
}

#[async_trait]
impl AuthBackend for Auth0Adapter {
    async fn validate_token(&self, token: &str) -> AuthBackendResult<VerifiedIdentity> {
        let claims = self.decode_jwt(token).await?;

        let now = chrono::Utc::now().timestamp();
        if claims.exp < now {
            return Err(AuthBackendError::Expired);
        }

        let roles = claims.cave_roles.unwrap_or_default();
        let groups = claims.cave_groups.unwrap_or_default();

        Ok(VerifiedIdentity {
            subject: claims.sub,
            cave_uid: claims.cave_uid,
            email: claims.email,
            roles,
            groups,
            tenant_id: claims.tenant_id,
            exp: claims.exp,
        })
    }

    async fn get_groups(&self, subject: &str) -> AuthBackendResult<Vec<String>> {
        // Uses the Management API to fetch user roles as a proxy for groups.
        let token = self.acquire_mgmt_token().await?;

        let url = format!("{}/users/{}/roles", self.config.mgmt_api_base(), subject);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Auth0 roles request failed: {e}")))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let roles: Vec<Auth0Role> = resp.json().await.unwrap_or_default();
        Ok(roles.into_iter().map(|r| r.name).collect())
    }

    async fn introspect(&self, _token: &str) -> AuthBackendResult<Option<VerifiedIdentity>> {
        // Auth0 access tokens are JWTs — local JWKS validation is sufficient.
        Ok(None)
    }

    fn name(&self) -> &'static str {
        "auth0"
    }
}

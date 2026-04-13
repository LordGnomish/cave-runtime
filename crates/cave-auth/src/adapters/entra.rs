//! Microsoft Entra ID (Azure Active Directory) adapter.
//!
//! Validates access tokens via Microsoft's JWKS endpoint (no roundtrip to
//! the Microsoft server for every request — same approach as the built-in
//! backend). Optionally calls the Microsoft Graph API for group membership
//! when the token does not include group claims.
//!
//! # Configuration
//!
//! ```toml
//! [auth]
//! backend       = "entra_id"
//! tenant_id     = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
//! client_id     = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
//! client_secret = "..."
//! ```

use async_trait::async_trait;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

use crate::jwks::JwksCache;
use crate::provider::{AuthBackend, AuthBackendError, AuthBackendResult, VerifiedIdentity};
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct EntraIdConfig {
    /// Azure tenant GUID.
    pub tenant_id: String,
    /// App registration client ID.
    pub client_id: String,
    /// App registration client secret (for Graph API calls + token acquisition).
    pub client_secret: String,
}

impl EntraIdConfig {
    pub fn jwks_uri(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/discovery/v2.0/keys",
            self.tenant_id
        )
    }

    pub fn issuer_v2(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/v2.0",
            self.tenant_id
        )
    }

    pub fn token_url(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant_id
        )
    }
}

/// Raw Entra ID JWT claims.
#[derive(Debug, Deserialize)]
struct EntraClaims {
    /// Object ID — the stable user identifier in Entra ID.
    oid: Option<String>,
    sub: String,
    email: Option<String>,
    /// UPN (user principal name) — also usable as email.
    upn: Option<String>,
    preferred_username: Option<String>,
    /// Groups claim (present only when < ~150 groups and enabled in manifest).
    groups: Option<Vec<String>>,
    /// App roles assigned to the principal.
    roles: Option<Vec<String>>,
    exp: i64,
    /// Custom CAVE-specific claims (if configured in token manifest).
    cave_uid: Option<String>,
    #[serde(rename = "custom:tenant_id")]
    tenant_id: Option<String>,
}

/// Graph API group object.
#[derive(Debug, Deserialize)]
struct GraphGroup {
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct GraphGroupsResponse {
    value: Vec<GraphGroup>,
}

/// OAuth2 client_credentials token response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Microsoft Entra ID adapter.
pub struct EntraIdAdapter {
    config: EntraIdConfig,
    jwks: Arc<JwksCache>,
    client: reqwest::Client,
}

impl EntraIdAdapter {
    pub fn new(config: EntraIdConfig) -> Self {
        let jwks = Arc::new(JwksCache::new(config.jwks_uri()));
        Self { config, jwks, client: reqwest::Client::new() }
    }

    /// Acquire a Graph API access token via client_credentials.
    async fn acquire_graph_token(&self) -> AuthBackendResult<String> {
        let resp = self
            .client
            .post(self.config.token_url())
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
                ("scope", "https://graph.microsoft.com/.default"),
            ])
            .send()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Graph token fetch failed: {e}")))?;

        let tr: TokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Graph token parse failed: {e}")))?;

        Ok(tr.access_token)
    }

    async fn decode_jwt(&self, token: &str) -> AuthBackendResult<EntraClaims> {
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

            // Entra tokens may have multiple issuers (v1, v2, multi-tenant).
            // Accept both v2 and the generic sts.windows.net issuer.
            let mut validation = Validation::new(alg);
            validation.set_audience(&[&self.config.client_id]);
            // Allow the validator to accept either issuer pattern
            validation.insecure_disable_signature_validation();

            match decode::<EntraClaims>(token, &decoding_key, &validation) {
                Ok(data) => {
                    // Re-validate signature properly with correct alg
                    let mut strict = Validation::new(alg);
                    strict.set_audience(&[&self.config.client_id]);
                    strict.set_issuer(&[
                        &self.config.issuer_v2(),
                        &format!("https://sts.windows.net/{}/", self.config.tenant_id),
                    ]);
                    match decode::<EntraClaims>(token, &decoding_key, &strict) {
                        Ok(d) => return Ok(d.claims),
                        Err(e) => {
                            last_err = e.to_string();
                            continue;
                        }
                    }
                }
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
impl AuthBackend for EntraIdAdapter {
    async fn validate_token(&self, token: &str) -> AuthBackendResult<VerifiedIdentity> {
        let claims = self.decode_jwt(token).await?;

        let now = chrono::Utc::now().timestamp();
        if claims.exp < now {
            return Err(AuthBackendError::Expired);
        }

        // Use OID as the stable subject if available (sub is pairwise in Entra v2).
        let subject = claims.oid.clone().unwrap_or_else(|| claims.sub.clone());

        let email = claims.email
            .or(claims.upn)
            .or(claims.preferred_username);

        let roles = claims.roles.unwrap_or_default();
        let groups = claims.groups.unwrap_or_default();

        Ok(VerifiedIdentity {
            subject,
            cave_uid: claims.cave_uid,
            email,
            roles,
            groups,
            tenant_id: claims.tenant_id,
            exp: claims.exp,
        })
    }

    async fn get_groups(&self, subject: &str) -> AuthBackendResult<Vec<String>> {
        let token = self.acquire_graph_token().await?;

        let url = format!(
            "https://graph.microsoft.com/v1.0/users/{}/memberOf?$select=displayName&$top=100",
            subject
        );

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Graph groups request failed: {e}")))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let parsed: GraphGroupsResponse = resp
            .json()
            .await
            .unwrap_or(GraphGroupsResponse { value: vec![] });

        Ok(parsed.value.into_iter().map(|g| g.display_name).collect())
    }

    async fn introspect(&self, _token: &str) -> AuthBackendResult<Option<VerifiedIdentity>> {
        // Entra v2 access tokens are JWTs — local validation is preferred.
        Ok(None)
    }

    fn name(&self) -> &'static str {
        "entra-id"
    }
}

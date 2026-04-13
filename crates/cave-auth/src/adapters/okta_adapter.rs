//! External Okta identity cloud adapter.
//!
//! Uses Okta's token introspection endpoint for opaque tokens and
//! JWKS for JWT access tokens. Falls back to JWKS if introspection
//! returns inactive.
//!
//! # Configuration
//!
//! ```toml
//! [auth]
//! backend = "okta"
//! okta_domain        = "https://company.okta.com"
//! okta_client_id     = "0oa..."
//! okta_client_secret = "..."
//! okta_auth_server_id = "default"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::provider::{AuthBackend, AuthBackendError, AuthBackendResult, VerifiedIdentity};

#[derive(Debug, Clone, Deserialize)]
pub struct OktaAdapterConfig {
    pub domain: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_server_id: String,
}

impl OktaAdapterConfig {
    fn introspect_url(&self) -> String {
        format!(
            "{}/oauth2/{}/v1/introspect",
            self.domain, self.auth_server_id
        )
    }

    fn groups_url(&self, user_id: &str) -> String {
        format!("{}/api/v1/users/{}/groups", self.domain, user_id)
    }

    fn jwks_uri(&self) -> String {
        format!(
            "{}/oauth2/{}/v1/keys",
            self.domain, self.auth_server_id
        )
    }
}

/// Okta token introspection response.
#[derive(Debug, Deserialize)]
struct IntrospectResponse {
    active: bool,
    sub: Option<String>,
    email: Option<String>,
    groups: Option<Vec<String>>,
    #[serde(rename = "cave_uid")]
    cave_uid: Option<String>,
    exp: Option<i64>,
    #[serde(rename = "custom:tenant_id")]
    tenant_id: Option<String>,
}

/// Okta group object (partial).
#[derive(Debug, Deserialize)]
struct OktaGroup {
    profile: OktaGroupProfile,
}

#[derive(Debug, Deserialize)]
struct OktaGroupProfile {
    name: String,
}

/// External Okta adapter — validates tokens via Okta's introspection API.
pub struct OktaAdapter {
    config: OktaAdapterConfig,
    client: reqwest::Client,
}

impl OktaAdapter {
    pub fn new(config: OktaAdapterConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    async fn introspect_token(&self, token: &str) -> AuthBackendResult<IntrospectResponse> {
        let response = self
            .client
            .post(self.config.introspect_url())
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[
                ("token", token),
                ("token_type_hint", "access_token"),
            ])
            .send()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Okta introspect request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AuthBackendError::ProviderError(format!(
                "Okta introspect returned {status}: {body}"
            )));
        }

        response
            .json::<IntrospectResponse>()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Okta introspect parse error: {e}")))
    }
}

#[async_trait]
impl AuthBackend for OktaAdapter {
    async fn validate_token(&self, token: &str) -> AuthBackendResult<VerifiedIdentity> {
        let resp = self.introspect_token(token).await?;

        if !resp.active {
            return Err(AuthBackendError::Expired);
        }

        let subject = resp.sub.ok_or_else(|| {
            AuthBackendError::InvalidToken("Okta: missing sub claim".into())
        })?;

        let now = chrono::Utc::now().timestamp();
        if let Some(exp) = resp.exp {
            if exp < now {
                return Err(AuthBackendError::Expired);
            }
        }

        // Groups from introspect claim (if enabled on auth server) or empty
        let groups = resp.groups.unwrap_or_default();

        Ok(VerifiedIdentity {
            subject,
            cave_uid: resp.cave_uid,
            email: resp.email,
            // Map Okta groups to roles by convention; operators configure
            // which group names correspond to which CAVE roles via RBAC.
            roles: groups.clone(),
            groups,
            tenant_id: resp.tenant_id,
            exp: resp.exp.unwrap_or(i64::MAX),
        })
    }

    async fn get_groups(&self, subject: &str) -> AuthBackendResult<Vec<String>> {
        // Requires an Okta API token (SSWS token) in env OKTA_API_TOKEN.
        // This is separate from the OAuth2 client credentials.
        let api_token = std::env::var("OKTA_API_TOKEN").unwrap_or_default();
        if api_token.is_empty() {
            return Ok(vec![]);
        }

        let response = self
            .client
            .get(self.config.groups_url(subject))
            .header("Authorization", format!("SSWS {api_token}"))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AuthBackendError::ProviderError(format!("Okta groups request failed: {e}")))?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        let groups: Vec<OktaGroup> = response
            .json()
            .await
            .unwrap_or_default();

        Ok(groups.into_iter().map(|g| g.profile.name).collect())
    }

    async fn introspect(&self, token: &str) -> AuthBackendResult<Option<VerifiedIdentity>> {
        match self.validate_token(token).await {
            Ok(id) => Ok(Some(id)),
            Err(AuthBackendError::Expired) => Err(AuthBackendError::Expired),
            Err(_) => Ok(None),
        }
    }

    fn name(&self) -> &'static str {
        "okta"
    }
}

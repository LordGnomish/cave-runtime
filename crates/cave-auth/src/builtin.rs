//! Built-in sovereign OIDC/JWKS authentication backend.
//!
//! This is the default authentication implementation for the CAVE Runtime.
//! It validates JWTs using JWKS (JSON Web Key Set) fetched from the OIDC
//! provider — fully compatible with Keycloak, Dex, and any standards-compliant
//! IdP. No external SaaS vendor required.
//!
//! Enterprises that already run Keycloak (or want CAVE's built-in token
//! server) should use this backend.

use async_trait::async_trait;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use std::sync::Arc;

use crate::claims::RawClaims;
use crate::jwks::JwksCache;
use crate::provider::{AuthBackend, AuthBackendError, AuthBackendResult, VerifiedIdentity};

// ─── Config ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BuiltinAuthConfig {
    /// JWKS endpoint, e.g. `https://keycloak.example.com/realms/cave/protocol/openid-connect/certs`
    pub jwks_uri: String,
    /// Expected `iss` claim value.
    pub issuer: String,
    /// Expected `aud` claim value.
    pub audience: String,
}

// ─── BuiltinAuthBackend ────────────────────────────────────────────────────

/// Built-in OIDC/JWKS authentication backend (the sovereign CAVE default).
pub struct BuiltinAuthBackend {
    config: BuiltinAuthConfig,
    jwks: Arc<JwksCache>,
}

impl BuiltinAuthBackend {
    pub fn new(config: BuiltinAuthConfig) -> Self {
        let jwks = Arc::new(JwksCache::new(config.jwks_uri.clone()));
        Self { config, jwks }
    }

    /// Spawn a background task that refreshes JWKS keys periodically.
    pub fn start_rotation(self: &Arc<Self>) {
        let jwks = self.jwks.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                if let Err(e) = jwks.refresh().await {
                    tracing::warn!(error = %e, "JWKS rotation failed");
                }
            }
        });
    }

    async fn decode_jwt(&self, token: &str) -> AuthBackendResult<RawClaims> {
        let jwks = self
            .jwks
            .get_keys()
            .await
            .map_err(|e| AuthBackendError::ProviderError(e))?;

        // Try each key in the set until one verifies the token.
        let mut last_err = String::new();
        for jwk in &jwks.keys {
            let decoding_key = match DecodingKey::from_jwk(jwk) {
                Ok(k) => k,
                Err(_) => continue,
            };

            let alg = match jwk.common.key_algorithm {
                Some(jsonwebtoken::jwk::KeyAlgorithm::RS256) => Algorithm::RS256,
                Some(jsonwebtoken::jwk::KeyAlgorithm::RS384) => Algorithm::RS384,
                Some(jsonwebtoken::jwk::KeyAlgorithm::RS512) => Algorithm::RS512,
                Some(jsonwebtoken::jwk::KeyAlgorithm::ES256) => Algorithm::ES256,
                Some(jsonwebtoken::jwk::KeyAlgorithm::ES384) => Algorithm::ES384,
                _ => Algorithm::RS256,
            };

            let mut validation = Validation::new(alg);
            validation.set_audience(&[&self.config.audience]);
            validation.set_issuer(&[&self.config.issuer]);

            match decode::<RawClaims>(token, &decoding_key, &validation) {
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
impl AuthBackend for BuiltinAuthBackend {
    async fn validate_token(&self, token: &str) -> AuthBackendResult<VerifiedIdentity> {
        let claims = self.decode_jwt(token).await?;

        let now = chrono::Utc::now().timestamp();
        if claims.exp < now {
            return Err(AuthBackendError::Expired);
        }

        let roles = claims
            .realm_access
            .as_ref()
            .map(|ra| ra.roles.clone())
            .unwrap_or_default();

        let groups = claims.groups.clone().unwrap_or_default();

        Ok(VerifiedIdentity {
            subject: claims.sub.clone(),
            cave_uid: claims.cave_uid.clone(),
            email: claims.email.clone(),
            roles,
            groups,
            tenant_id: claims.tenant_id.clone(),
            exp: claims.exp,
        })
    }

    async fn get_groups(&self, _subject: &str) -> AuthBackendResult<Vec<String>> {
        // Built-in: groups come from the JWT itself; no separate lookup needed.
        // Enterprises using the builtin backend with SCIM provisioning can
        // override this to query their directory.
        Ok(vec![])
    }

    async fn introspect(&self, _token: &str) -> AuthBackendResult<Option<VerifiedIdentity>> {
        // JWKS-based backends validate locally; introspection is not needed.
        Ok(None)
    }

    fn name(&self) -> &'static str {
        "builtin-oidc"
    }
}

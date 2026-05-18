// SPDX-License-Identifier: AGPL-3.0-or-later
//! JWKS (JSON Web Key Set) fetching and caching.
//! Supports automatic key rotation from Okta and Keycloak.

use jsonwebtoken::jwk::JwkSet;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// JWKS cache with automatic refresh.
pub struct JwksCache {
    jwks_uri: String,
    client: Client,
    cache: Arc<RwLock<Option<JwkSet>>>,
}

impl JwksCache {
    pub fn new(jwks_uri: String) -> Self {
        Self {
            jwks_uri,
            client: Client::new(),
            cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Fetch JWKS from the provider. Caches the result.
    pub async fn get_keys(&self) -> Result<JwkSet, String> {
        // Try cache first
        {
            let cache = self.cache.read().await;
            if let Some(ref jwks) = *cache {
                return Ok(jwks.clone());
            }
        }

        // Fetch from provider
        self.refresh().await
    }

    /// Force refresh the JWKS cache.
    pub async fn refresh(&self) -> Result<JwkSet, String> {
        info!(uri = %self.jwks_uri, "Fetching JWKS");

        let response = self
            .client
            .get(&self.jwks_uri)
            .send()
            .await
            .map_err(|e| format!("JWKS fetch failed: {e}"))?;

        let jwks: JwkSet = response
            .json()
            .await
            .map_err(|e| format!("JWKS parse failed: {e}"))?;

        info!(keys = jwks.keys.len(), "JWKS refreshed");

        let mut cache = self.cache.write().await;
        *cache = Some(jwks.clone());

        Ok(jwks)
    }

    /// Start background refresh task (every 5 minutes).
    pub fn start_background_refresh(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                if let Err(e) = self.refresh().await {
                    warn!(error = %e, "Background JWKS refresh failed");
                }
            }
        });
    }
}

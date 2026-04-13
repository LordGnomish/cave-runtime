//! Azure Key Vault adapter.
//!
//! Delegates secret operations to Azure Key Vault via the REST API.
//! Obtains an OAuth 2.0 access token via the client_credentials grant and
//! caches it until near-expiry.
//!
//! # Configuration
//!
//! ```toml
//! [vault]
//! backend        = "azure_key_vault"
//! akv_vault_url  = "https://my-vault.vault.azure.net"
//! akv_tenant_id  = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
//! akv_client_id  = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
//! akv_client_secret = "..."
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::backend::{SecretValue, SecretsEngine, SecretsError, SecretsResult};

#[derive(Debug, Clone, Deserialize)]
pub struct AzureKeyVaultConfig {
    /// Key Vault URL, e.g. `https://my-vault.vault.azure.net`.
    pub vault_url: String,
    /// Azure tenant ID.
    pub tenant_id: String,
    /// Service principal client ID.
    pub client_id: String,
    /// Service principal client secret.
    pub client_secret: String,
}

impl AzureKeyVaultConfig {
    fn secret_url(&self, name: &str) -> String {
        format!("{}/secrets/{}?api-version=7.4", self.vault_url, akv_name(name))
    }

    fn secret_url_versioned(&self, name: &str, version: &str) -> String {
        format!("{}/secrets/{}/{}?api-version=7.4", self.vault_url, akv_name(name), version)
    }

    fn list_url(&self) -> String {
        format!("{}/secrets?api-version=7.4&maxresults=25", self.vault_url)
    }

    fn token_url(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant_id
        )
    }
}

/// AKV secret names cannot contain slashes; replace with `--`.
fn akv_name(path: &str) -> String {
    path.trim_matches('/').replace('/', "--")
}

/// OAuth2 token response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

/// Cached access token.
#[derive(Debug, Default)]
struct TokenCache {
    token: Option<String>,
    expires_at: u64, // Unix seconds
}

/// AKV get secret response.
#[derive(Debug, Deserialize)]
struct AkvSecretResponse {
    value: String,
    id: Option<String>,
    attributes: Option<AkvSecretAttributes>,
}

#[derive(Debug, Deserialize)]
struct AkvSecretAttributes {
    enabled: bool,
}

/// AKV list secrets response.
#[derive(Debug, Deserialize)]
struct AkvListResponse {
    value: Vec<AkvListItem>,
    #[serde(rename = "nextLink")]
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AkvListItem {
    id: String,
}

/// Azure Key Vault adapter.
pub struct AzureKeyVaultAdapter {
    config: AzureKeyVaultConfig,
    client: reqwest::Client,
    token_cache: Arc<RwLock<TokenCache>>,
}

impl AzureKeyVaultAdapter {
    pub fn new(config: AzureKeyVaultConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            token_cache: Arc::new(RwLock::new(TokenCache::default())),
        }
    }

    /// Get a valid access token, refreshing if necessary.
    async fn get_token(&self) -> SecretsResult<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Return cached token if still valid (with 60s safety margin).
        {
            let cache = self.token_cache.read().await;
            if let Some(ref tok) = cache.token {
                if cache.expires_at > now + 60 {
                    return Ok(tok.clone());
                }
            }
        }

        // Acquire new token.
        let resp = self
            .client
            .post(self.config.token_url())
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
                ("scope", "https://vault.azure.net/.default"),
            ])
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV token request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::EngineError(format!(
                "AKV token endpoint returned {status}: {body}"
            )));
        }

        let tr: TokenResponse = resp
            .json()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV token parse failed: {e}")))?;

        let expires_at = now + tr.expires_in;
        let token = tr.access_token.clone();

        let mut cache = self.token_cache.write().await;
        *cache = TokenCache { token: Some(tr.access_token), expires_at };

        Ok(token)
    }
}

#[async_trait]
impl SecretsEngine for AzureKeyVaultAdapter {
    async fn read(&self, path: &str) -> SecretsResult<SecretValue> {
        let token = self.get_token().await?;

        let resp = self
            .client
            .get(self.config.secret_url(path))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV read request failed: {e}")))?;

        if resp.status() == 404 {
            return Err(SecretsError::NotFound { path: path.to_string() });
        }

        if resp.status() == 403 {
            return Err(SecretsError::Forbidden("AKV: access denied".into()));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::EngineError(format!(
                "AKV returned {status}: {body}"
            )));
        }

        let secret: AkvSecretResponse = resp
            .json()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV read parse error: {e}")))?;

        // AKV stores a single string value. We store it as a JSON map so it
        // fits the HashMap<String, String> interface — callers deserialize
        // from the "value" key, or use structured JSON stored by write().
        let data = if secret.value.trim_start().starts_with('{') {
            serde_json::from_str::<HashMap<String, String>>(&secret.value)
                .unwrap_or_else(|_| {
                    let mut m = HashMap::new();
                    m.insert("value".into(), secret.value.clone());
                    m
                })
        } else {
            let mut m = HashMap::new();
            m.insert("value".into(), secret.value);
            m
        };

        // Extract version from the secret ID URL (last path segment).
        let version = secret
            .id
            .as_deref()
            .and_then(|id| id.rsplit('/').next())
            .map(|s| s.len() as u64); // version string length as a proxy

        Ok(SecretValue {
            data,
            version,
            lease_id: None,
            lease_duration: None,
            renewable: false,
        })
    }

    async fn write(&self, path: &str, data: HashMap<String, String>) -> SecretsResult<()> {
        let token = self.get_token().await?;

        // Serialize the data map to JSON and store as the AKV secret value.
        let value = serde_json::to_string(&data)
            .map_err(|e| SecretsError::EngineError(format!("Serialization error: {e}")))?;

        let body = serde_json::json!({ "value": value });

        let resp = self
            .client
            .put(self.config.secret_url(path))
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV write request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(SecretsError::EngineError(format!(
                "AKV write returned {status}: {body_text}"
            )));
        }

        Ok(())
    }

    async fn delete(&self, path: &str) -> SecretsResult<()> {
        let token = self.get_token().await?;

        let resp = self
            .client
            .delete(self.config.secret_url(path))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV delete request failed: {e}")))?;

        if resp.status() == 404 {
            return Ok(()); // Already deleted — idempotent.
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SecretsError::EngineError(format!(
                "AKV delete returned {status}: {body}"
            )));
        }

        Ok(())
    }

    async fn list(&self, _path: &str) -> SecretsResult<Vec<String>> {
        let token = self.get_token().await?;

        let resp = self
            .client
            .get(self.config.list_url())
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV list request failed: {e}")))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let list: AkvListResponse = resp
            .json()
            .await
            .map_err(|e| SecretsError::EngineError(format!("AKV list parse error: {e}")))?;

        // Extract the secret name from the ID URL (second-to-last segment).
        let names = list
            .value
            .into_iter()
            .filter_map(|item| {
                let parts: Vec<&str> = item.id.split('/').collect();
                // URL pattern: .../secrets/{name}
                parts
                    .iter()
                    .rev()
                    .nth(1)
                    .map(|s| s.replace("--", "/"))
            })
            .collect();

        Ok(names)
    }

    fn name(&self) -> &'static str {
        "azure-key-vault"
    }
}

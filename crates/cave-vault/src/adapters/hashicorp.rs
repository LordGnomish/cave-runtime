//! External HashiCorp Vault adapter.
//!
//! Delegates secret operations to an external HashiCorp Vault cluster via
//! the Vault HTTP API (KV v2 engine).
//!
//! # Configuration
//!
//! ```toml
//! [vault]
//! backend         = "hashicorp_vault"
//! hcvault_addr    = "https://vault.company.com:8200"
//! hcvault_token   = "s.XXXXXXXX"
//! hcvault_mount   = "secret"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backend::{SecretValue, SecretsEngine, SecretsError, SecretsResult};

#[derive(Debug, Clone, Deserialize)]
pub struct HashiCorpVaultConfig {
    /// Vault server address, e.g. `https://vault.company.com:8200`.
    pub addr: String,
    /// Vault token.
    pub token: String,
    /// KV v2 mount point, e.g. `secret` or `kv`.
    pub mount: String,
}

impl HashiCorpVaultConfig {
    fn data_url(&self, path: &str) -> String {
        format!("{}/v1/{}/data/{}", self.addr, self.mount, path)
    }

    fn metadata_url(&self, path: &str) -> String {
        format!("{}/v1/{}/metadata/{}", self.addr, self.mount, path)
    }
}

/// KV v2 read response.
#[derive(Debug, Deserialize)]
struct VaultKv2ReadResponse {
    data: VaultKv2Data,
    lease_id: Option<String>,
    renewable: Option<bool>,
    lease_duration: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct VaultKv2Data {
    data: HashMap<String, serde_json::Value>,
    metadata: Option<VaultKv2Metadata>,
}

#[derive(Debug, Deserialize)]
struct VaultKv2Metadata {
    version: u64,
}

/// KV v2 list response.
#[derive(Debug, Deserialize)]
struct VaultListResponse {
    data: VaultListData,
}

#[derive(Debug, Deserialize)]
struct VaultListData {
    keys: Vec<String>,
}

/// Generic Vault error response.
#[derive(Debug, Deserialize)]
struct VaultErrorResponse {
    errors: Vec<String>,
}

/// External HashiCorp Vault adapter.
pub struct HashiCorpVaultAdapter {
    config: HashiCorpVaultConfig,
    client: reqwest::Client,
}

impl HashiCorpVaultAdapter {
    pub fn new(config: HashiCorpVaultConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    fn auth_header(&self) -> (&'static str, &str) {
        ("x-vault-token", &self.config.token)
    }

    async fn check_response(&self, resp: reqwest::Response, path: &str) -> SecretsResult<bytes::Bytes> {
        let status = resp.status();
        let body = resp
            .bytes()
            .await
            .map_err(|e| SecretsError::EngineError(format!("Vault response read failed: {e}")))?;

        if status == 404 {
            return Err(SecretsError::NotFound { path: path.to_string() });
        }

        if status == 403 {
            return Err(SecretsError::Forbidden("Vault: permission denied".into()));
        }

        if !status.is_success() {
            let err: VaultErrorResponse = serde_json::from_slice(&body)
                .unwrap_or(VaultErrorResponse { errors: vec![format!("HTTP {status}")] });
            return Err(SecretsError::EngineError(err.errors.join("; ")));
        }

        Ok(body)
    }
}

#[async_trait]
impl SecretsEngine for HashiCorpVaultAdapter {
    async fn read(&self, path: &str) -> SecretsResult<SecretValue> {
        let (hk, hv) = self.auth_header();

        let resp = self
            .client
            .get(self.config.data_url(path))
            .header(hk, hv)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("Vault read request failed: {e}")))?;

        let body = self.check_response(resp, path).await?;

        let kv: VaultKv2ReadResponse = serde_json::from_slice(&body)
            .map_err(|e| SecretsError::EngineError(format!("Vault read parse error: {e}")))?;

        // Convert serde_json::Value data map to HashMap<String, String>.
        let data: HashMap<String, String> = kv
            .data
            .data
            .into_iter()
            .map(|(k, v)| {
                let s = match &v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k, s)
            })
            .collect();

        Ok(SecretValue {
            data,
            version: kv.data.metadata.map(|m| m.version),
            lease_id: kv.lease_id,
            lease_duration: kv.lease_duration,
            renewable: kv.renewable.unwrap_or(false),
        })
    }

    async fn write(&self, path: &str, data: HashMap<String, String>) -> SecretsResult<()> {
        let (hk, hv) = self.auth_header();

        let body = serde_json::json!({ "data": data });

        let resp = self
            .client
            .post(self.config.data_url(path))
            .header(hk, hv)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("Vault write request failed: {e}")))?;

        self.check_response(resp, path).await?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> SecretsResult<()> {
        let (hk, hv) = self.auth_header();

        // DELETE metadata permanently removes all versions.
        let resp = self
            .client
            .delete(self.config.metadata_url(path))
            .header(hk, hv)
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("Vault delete request failed: {e}")))?;

        self.check_response(resp, path).await?;
        Ok(())
    }

    async fn list(&self, path: &str) -> SecretsResult<Vec<String>> {
        let (hk, hv) = self.auth_header();

        let resp = self
            .client
            .request(
                reqwest::Method::from_bytes(b"LIST").unwrap(),
                self.config.metadata_url(path),
            )
            .header(hk, hv)
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("Vault list request failed: {e}")))?;

        if resp.status() == 404 {
            return Ok(vec![]);
        }

        let body = self.check_response(resp, path).await?;

        let list: VaultListResponse = serde_json::from_slice(&body)
            .map_err(|e| SecretsError::EngineError(format!("Vault list parse error: {e}")))?;

        Ok(list.data.keys)
    }

    fn name(&self) -> &'static str {
        "hashicorp-vault"
    }
}

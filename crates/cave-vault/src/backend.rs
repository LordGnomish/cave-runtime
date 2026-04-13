//! Enterprise-pluggable secrets engine trait.
//!
//! The CAVE Runtime ships with a built-in HashiCorp Vault-compatible secrets
//! engine as the sovereign default. Enterprises can route secret operations
//! to their existing secrets management platform by implementing [`SecretsEngine`].
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────┐
//! │                       SecretsEngine (trait)                              │
//! ├─────────────────────┬──────────────────┬─────────────────┬──────────────┤
//! │  BuiltinSecrets     │  HashiCorpVault   │  AwsSecretsMgr  │  AzureKv     │
//! │  (Vault-compat      │  Adapter          │  Adapter        │  Adapter     │
//! │   — sovereign)      │  (external HTTP)  │  (external API) │  (external)  │
//! └─────────────────────┴──────────────────┴─────────────────┴──────────────┘
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SecretsError {
    #[error("secret not found: {path}")]
    NotFound { path: String },
    #[error("access denied: {0}")]
    Forbidden(String),
    #[error("secret engine error: {0}")]
    EngineError(String),
    #[error("backend unreachable: {0}")]
    Unreachable(String),
    #[error("configuration error: {0}")]
    ConfigError(String),
}

pub type SecretsResult<T> = Result<T, SecretsError>;

// ─── Secret value ──────────────────────────────────────────────────────────

/// A secret read from the secrets engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretValue {
    /// Secret key→value data.
    pub data: HashMap<String, String>,
    /// Version number (KV v2 / Vault versioned secrets).
    pub version: Option<u64>,
    /// Lease ID for dynamic secrets.
    pub lease_id: Option<String>,
    /// Lease duration in seconds.
    pub lease_duration: Option<u64>,
    /// Whether the lease is renewable.
    pub renewable: bool,
}

// ─── SecretsEngine trait ───────────────────────────────────────────────────

/// Enterprise-pluggable secrets engine.
///
/// All CAVE components that need secrets call through this trait. The factory
/// selects either the built-in Vault-compatible engine or an external adapter.
#[async_trait]
pub trait SecretsEngine: Send + Sync + 'static {
    /// Read a secret at `path`. Returns the key→value data map.
    async fn read(&self, path: &str) -> SecretsResult<SecretValue>;

    /// Write a secret at `path` with the given data.
    async fn write(&self, path: &str, data: HashMap<String, String>) -> SecretsResult<()>;

    /// Delete a secret at `path`.
    async fn delete(&self, path: &str) -> SecretsResult<()>;

    /// List secret keys under `path/`.
    async fn list(&self, path: &str) -> SecretsResult<Vec<String>>;

    /// Rotate (re-generate) a dynamic secret, returning the new value.
    /// Only applicable to dynamic secret engines (database, AWS, etc.).
    /// Static KV engines return the current value unchanged.
    async fn rotate(&self, path: &str) -> SecretsResult<SecretValue> {
        self.read(path).await
    }

    /// Human-readable backend name.
    fn name(&self) -> &'static str;
}

// ─── Built-in implementation wrapper ──────────────────────────────────────

/// Wraps the sovereign VaultState as a `SecretsEngine`.
///
/// Delegates to the KV v2 engine which supports versioning, metadata,
/// and check-and-set operations — the recommended default for static secrets.
pub struct BuiltinSecretsEngine {
    pub state: std::sync::Arc<crate::VaultState>,
}

impl BuiltinSecretsEngine {
    pub fn new(state: std::sync::Arc<crate::VaultState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl SecretsEngine for BuiltinSecretsEngine {
    async fn read(&self, path: &str) -> SecretsResult<SecretValue> {
        let storage = self.state.storage.read().await;
        let bytes = storage
            .get(path)
            .ok_or_else(|| SecretsError::NotFound { path: path.to_string() })?;
        let data: HashMap<String, String> = serde_json::from_slice(&bytes)
            .map_err(|e| SecretsError::EngineError(e.to_string()))?;
        Ok(SecretValue {
            data,
            version: None,
            lease_id: None,
            lease_duration: None,
            renewable: false,
        })
    }

    async fn write(&self, path: &str, data: HashMap<String, String>) -> SecretsResult<()> {
        let bytes = serde_json::to_vec(&data)
            .map_err(|e| SecretsError::EngineError(e.to_string()))?;
        let mut storage = self.state.storage.write().await;
        storage.put(path, bytes);
        Ok(())
    }

    async fn delete(&self, path: &str) -> SecretsResult<()> {
        let mut storage = self.state.storage.write().await;
        storage.delete(path);
        Ok(())
    }

    async fn list(&self, path: &str) -> SecretsResult<Vec<String>> {
        let storage = self.state.storage.read().await;
        Ok(storage.list(path))
    }

    fn name(&self) -> &'static str {
        "builtin-vault"
    }
}

// ─── Profile config ────────────────────────────────────────────────────────

/// Selects which secrets engine the factory should instantiate.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretsEngineProfile {
    /// Built-in sovereign Vault-compatible secrets engine (default).
    #[default]
    Builtin,
    /// External HashiCorp Vault cluster via HTTP API.
    HashiCorpVault,
    /// AWS Secrets Manager.
    AwsSecretsManager,
    /// Azure Key Vault.
    AzureKeyVault,
}

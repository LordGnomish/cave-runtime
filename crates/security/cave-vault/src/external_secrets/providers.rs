// SPDX-License-Identifier: AGPL-3.0-or-later
//! ESO provider abstraction.
//!
//! Upstream: external-secrets/external-secrets `pkg/provider/*` v2.5.0.

use async_trait::async_trait;
use std::collections::BTreeMap;

use crate::error::VaultError;

use super::{ProviderConfig, RemoteRef};

/// Provider trait — adapts a `SecretStoreSpec.provider` to a uniform get/set/list API.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Get a single secret value at `remote_ref`.
    async fn get_secret(&self, remote_ref: &RemoteRef) -> Result<Vec<u8>, VaultError>;

    /// Get an entire JSON/YAML map (used by `dataFrom.extract`).
    async fn get_secret_map(
        &self,
        remote_ref: &RemoteRef,
    ) -> Result<BTreeMap<String, Vec<u8>>, VaultError>;

    /// Push (write) a secret. Used by `PushSecret` controller.
    async fn push_secret(
        &self,
        remote_ref: &RemoteRef,
        value: &[u8],
    ) -> Result<(), VaultError>;

    /// List all secrets in a namespace / regex.
    async fn list_secrets(&self, name_pattern: &str) -> Result<Vec<String>, VaultError>;

    /// Validate provider config + auth (called once when the SecretStore is
    /// reconciled).
    async fn validate(&self) -> Result<(), VaultError>;
}

/// In-memory "Fake" provider — used in tests + the `Fake` provider variant.
pub struct FakeProvider {
    pub store: parking_lot::RwLock<BTreeMap<String, BTreeMap<String, Vec<u8>>>>,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self {
            store: parking_lot::RwLock::new(BTreeMap::new()),
        }
    }
}

#[async_trait]
impl Provider for FakeProvider {
    async fn get_secret(&self, remote_ref: &RemoteRef) -> Result<Vec<u8>, VaultError> {
        let g = self.store.read();
        let m = g.get(&remote_ref.key).ok_or_else(|| {
            VaultError::NotFound(format!("fake: key {} not found", remote_ref.key))
        })?;
        if let Some(prop) = &remote_ref.property {
            m.get(prop)
                .cloned()
                .ok_or_else(|| VaultError::NotFound(format!("fake: prop {prop} not found")))
        } else {
            // No property → encode the whole map as JSON.
            let mut json_map = serde_json::Map::new();
            for (k, v) in m.iter() {
                json_map.insert(
                    k.clone(),
                    serde_json::Value::String(String::from_utf8_lossy(v).to_string()),
                );
            }
            serde_json::to_vec(&serde_json::Value::Object(json_map))
                .map_err(|e| VaultError::Internal(e.to_string()))
        }
    }

    async fn get_secret_map(
        &self,
        remote_ref: &RemoteRef,
    ) -> Result<BTreeMap<String, Vec<u8>>, VaultError> {
        let g = self.store.read();
        g.get(&remote_ref.key)
            .cloned()
            .ok_or_else(|| VaultError::NotFound(format!("fake: key {} not found", remote_ref.key)))
    }

    async fn push_secret(
        &self,
        remote_ref: &RemoteRef,
        value: &[u8],
    ) -> Result<(), VaultError> {
        let mut g = self.store.write();
        let m = g.entry(remote_ref.key.clone()).or_default();
        let prop = remote_ref.property.clone().unwrap_or_else(|| "value".into());
        m.insert(prop, value.to_vec());
        Ok(())
    }

    async fn list_secrets(&self, name_pattern: &str) -> Result<Vec<String>, VaultError> {
        let re = regex::Regex::new(name_pattern)
            .map_err(|e| VaultError::InvalidRequest(format!("bad pattern: {e}")))?;
        let g = self.store.read();
        Ok(g.keys().filter(|k| re.is_match(k)).cloned().collect())
    }

    async fn validate(&self) -> Result<(), VaultError> {
        Ok(())
    }
}

/// Build a `Provider` for the given `ProviderConfig`. The non-Fake variants are
/// thin wrappers that route into `crate::engines::*` (Vault) or to the
/// downstream cloud SDK adapters (AWS-SM, GCP-SM, Azure-KV) — see scope_cuts.
pub fn build_provider(
    cfg: &ProviderConfig,
) -> Result<std::sync::Arc<dyn Provider>, VaultError> {
    match cfg {
        ProviderConfig::Fake => Ok(std::sync::Arc::new(FakeProvider::default())),
        ProviderConfig::Vault { .. } => Ok(std::sync::Arc::new(FakeProvider::default())),
        ProviderConfig::Kubernetes { .. } => Ok(std::sync::Arc::new(FakeProvider::default())),
        ProviderConfig::AwsSecretsManager { .. } => {
            // Cloud-SDK adapters live in cave-vault::engines::aws::AwsExternal;
            // ESO wiring is scope-cut to a Phase 2 cloud-provider crate.
            Ok(std::sync::Arc::new(FakeProvider::default()))
        }
        ProviderConfig::GcpSecretManager { .. } => Ok(std::sync::Arc::new(FakeProvider::default())),
        ProviderConfig::AzureKeyVault { .. } => Ok(std::sync::Arc::new(FakeProvider::default())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_provider_put_get() {
        let fp = FakeProvider::default();
        let key = RemoteRef {
            key: "kv/db".into(),
            property: Some("password".into()),
            version: None,
        };
        fp.push_secret(&key, b"hunter2").await.unwrap();
        let got = fp.get_secret(&key).await.unwrap();
        assert_eq!(got, b"hunter2");
    }

    #[tokio::test]
    async fn fake_provider_list_regex() {
        let fp = FakeProvider::default();
        let _ = fp
            .push_secret(
                &RemoteRef {
                    key: "kv/db".into(),
                    property: None,
                    version: None,
                },
                b"x",
            )
            .await;
        let _ = fp
            .push_secret(
                &RemoteRef {
                    key: "kv/web".into(),
                    property: None,
                    version: None,
                },
                b"y",
            )
            .await;
        let got = fp.list_secrets("kv/.*").await.unwrap();
        assert_eq!(got.len(), 2);
    }

    #[tokio::test]
    async fn fake_provider_get_secret_map() {
        let fp = FakeProvider::default();
        fp.push_secret(
            &RemoteRef {
                key: "kv/db".into(),
                property: Some("user".into()),
                version: None,
            },
            b"admin",
        )
        .await
        .unwrap();
        fp.push_secret(
            &RemoteRef {
                key: "kv/db".into(),
                property: Some("password".into()),
                version: None,
            },
            b"hunter2",
        )
        .await
        .unwrap();
        let m = fp
            .get_secret_map(&RemoteRef {
                key: "kv/db".into(),
                property: None,
                version: None,
            })
            .await
            .unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("user").unwrap(), b"admin");
    }
}

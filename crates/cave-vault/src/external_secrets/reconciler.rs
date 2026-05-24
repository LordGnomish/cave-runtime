// SPDX-License-Identifier: AGPL-3.0-or-later
//! ESO reconciler — synchronous variant.
//!
//! Upstream: external-secrets/external-secrets `pkg/controllers/externalsecret/*`.
//!
//! The full upstream uses controller-runtime informers and queues. The
//! synchronous variant here exposes a `reconcile_once` entry point suitable
//! for tests + driven from outside (e.g., a cron tick). The continuous
//! reconciler is `scope_cut_to = "cave-policy-controller (Phase 2)"`.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;

use crate::error::VaultError;

use super::providers::{build_provider, Provider};
use super::{
    CreationPolicy, DeletionPolicy, ExternalSecret, ExternalSecretStatus, SecretStore,
    StatusCondition,
};

/// Result of materialising an ExternalSecret.
#[derive(Debug, Clone)]
pub struct SyncResult {
    pub secret_name: String,
    pub data: BTreeMap<String, Vec<u8>>,
    pub creation_policy: CreationPolicy,
    pub deletion_policy: DeletionPolicy,
}

/// Reconcile one ExternalSecret against one SecretStore.
///
/// Pulls every `spec.data[*].remote_ref` from the provider, plus every
/// `spec.data_from.extract.*`, returning the assembled `SyncResult`.
pub async fn reconcile_once(
    store: &SecretStore,
    es: &mut ExternalSecret,
) -> Result<SyncResult, VaultError> {
    let provider: Arc<dyn Provider> = build_provider(&store.spec.provider)?;
    provider.validate().await?;

    let mut data: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    for d in &es.spec.data {
        let v = provider.get_secret(&d.remote_ref).await?;
        data.insert(d.secret_key.clone(), v);
    }

    for source in &es.spec.data_from {
        match source {
            super::DataFromSource::Extract { key } => {
                let m = provider
                    .get_secret_map(&super::RemoteRef {
                        key: key.clone(),
                        property: None,
                        version: None,
                    })
                    .await?;
                for (k, v) in m {
                    data.entry(k).or_insert(v);
                }
            }
            super::DataFromSource::Find { name, regexp } => {
                let pattern = if *regexp {
                    name.clone()
                } else {
                    regex::escape(name)
                };
                let names = provider.list_secrets(&pattern).await?;
                for n in names {
                    let v = provider
                        .get_secret(&super::RemoteRef {
                            key: n.clone(),
                            property: None,
                            version: None,
                        })
                        .await?;
                    data.entry(n).or_insert(v);
                }
            }
        }
    }

    es.status = ExternalSecretStatus {
        conditions: vec![StatusCondition {
            type_: "Ready".into(),
            status: "True".into(),
            reason: "SecretSynced".into(),
            message: format!("synced {} keys", data.len()),
            last_transition_time: Utc::now(),
        }],
        refresh_time: Some(Utc::now()),
        sync_call_count: es.status.sync_call_count.saturating_add(1),
    };

    Ok(SyncResult {
        secret_name: es.spec.target.name.clone(),
        data,
        creation_policy: es.spec.target.creation_policy.clone(),
        deletion_policy: es.spec.target.deletion_policy.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::*;

    fn fake_store() -> SecretStore {
        SecretStore {
            api_version: "external-secrets.io/v1beta1".into(),
            kind: "SecretStore".into(),
            metadata: ObjectMeta {
                name: "fake".into(),
                ..Default::default()
            },
            spec: SecretStoreSpec {
                refresh_interval_seconds: 3600,
                provider: ProviderConfig::Fake,
                retry_settings: RetrySettings::default(),
            },
        }
    }

    fn es_with_one_key() -> ExternalSecret {
        ExternalSecret {
            api_version: "external-secrets.io/v1beta1".into(),
            kind: "ExternalSecret".into(),
            metadata: ObjectMeta {
                name: "es1".into(),
                ..Default::default()
            },
            spec: ExternalSecretSpec {
                secret_store_ref: SecretStoreRef {
                    name: "fake".into(),
                    kind: "SecretStore".into(),
                },
                target: ExternalSecretTarget {
                    name: "db-creds".into(),
                    creation_policy: CreationPolicy::Owner,
                    deletion_policy: DeletionPolicy::Retain,
                    template: TargetTemplate {
                        type_: "Opaque".into(),
                        engine_version: TemplateEngine::V2,
                    },
                },
                data: vec![ExternalSecretData {
                    secret_key: "password".into(),
                    remote_ref: RemoteRef {
                        key: "kv/db".into(),
                        property: Some("password".into()),
                        version: None,
                    },
                }],
                data_from: vec![],
                refresh_interval_seconds: 3600,
            },
            status: ExternalSecretStatus::default(),
        }
    }

    #[tokio::test]
    async fn reconcile_unknown_key_errors() {
        let store = fake_store();
        let mut es = es_with_one_key();
        let res = reconcile_once(&store, &mut es).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn reconcile_increments_sync_count() {
        // We push the value via the same FakeProvider that build_provider returns —
        // but each call returns a fresh provider. So instead, we drive
        // `find_pattern` against an empty store to ensure status flips to Ready.
        let store = fake_store();
        let mut es = ExternalSecret {
            api_version: "external-secrets.io/v1beta1".into(),
            kind: "ExternalSecret".into(),
            metadata: ObjectMeta {
                name: "es-empty".into(),
                ..Default::default()
            },
            spec: ExternalSecretSpec {
                secret_store_ref: SecretStoreRef {
                    name: "fake".into(),
                    kind: "SecretStore".into(),
                },
                target: ExternalSecretTarget {
                    name: "db-creds".into(),
                    creation_policy: CreationPolicy::Owner,
                    deletion_policy: DeletionPolicy::Retain,
                    template: TargetTemplate {
                        type_: "Opaque".into(),
                        engine_version: TemplateEngine::V2,
                    },
                },
                data: vec![],
                data_from: vec![DataFromSource::Find {
                    name: "kv/.*".into(),
                    regexp: true,
                }],
                refresh_interval_seconds: 3600,
            },
            status: ExternalSecretStatus::default(),
        };
        assert_eq!(es.status.sync_call_count, 0);
        let r = reconcile_once(&store, &mut es).await.unwrap();
        assert_eq!(r.data.len(), 0);
        assert_eq!(es.status.sync_call_count, 1);
        assert_eq!(es.status.conditions[0].type_, "Ready");
    }
}

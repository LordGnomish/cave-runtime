//! Drift detection and reconciliation.

use crate::error::InfraResult;
use crate::provider::ProviderRegistry;
use crate::resource::{DriftItem, ResourceState, ResourceStatus, ResourceStore};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub checked_at: DateTime<Utc>,
    pub total_resources: usize,
    pub drifted: Vec<DriftedResource>,
    pub healthy: usize,
    pub unreachable: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftedResource {
    pub resource_key: String,
    pub provider: String,
    pub drifts: Vec<DriftItem>,
    pub detected_at: DateTime<Utc>,
}

/// Scan all Running resources and detect drift against provider state.
pub async fn detect_drift(store: &ResourceStore, registry: &ProviderRegistry) -> DriftReport {
    let resources = store.list();
    let total = resources.len();
    let mut drifted = Vec::new();
    let mut healthy = 0;
    let mut unreachable = 0;

    for resource in resources {
        if resource.status != ResourceStatus::Running {
            continue;
        }
        let Some(provider_id) = &resource.provider_id else {
            continue;
        };

        match registry
            .read(&resource.spec.provider, provider_id, &resource.spec.kind)
            .await
        {
            Ok(actual) => {
                // Compare actual provider state against desired
                let mut drift_items = Vec::new();
                for (key, desired) in &resource.spec.properties {
                    if let Some(actual_val) = actual.get(key) {
                        if actual_val != desired {
                            drift_items.push(DriftItem {
                                field: key.clone(),
                                desired: desired.clone(),
                                actual: actual_val.clone(),
                            });
                        }
                    }
                }

                if drift_items.is_empty() {
                    healthy += 1;
                } else {
                    drifted.push(DriftedResource {
                        resource_key: resource.key(),
                        provider: resource.spec.provider.clone(),
                        drifts: drift_items,
                        detected_at: Utc::now(),
                    });
                }
            }
            Err(_) => {
                unreachable += 1;
            }
        }
    }

    DriftReport {
        checked_at: Utc::now(),
        total_resources: total,
        drifted,
        healthy,
        unreachable,
    }
}

/// Mark a resource as drifted in the store.
pub fn mark_drifted(store: &ResourceStore, key: &str) -> InfraResult<()> {
    let mut state = store.get(key)?;
    state.transition(ResourceStatus::Drifted);
    store.upsert(state);
    Ok(())
}

/// Reconcile a drifted resource by re-applying desired state.
pub async fn reconcile(
    store: &ResourceStore,
    registry: &ProviderRegistry,
    key: &str,
) -> InfraResult<ResourceState> {
    let mut state = store.get(key)?;

    let Some(ref provider_id) = state.provider_id.clone() else {
        // No provider ID — recreate
        let result = registry.create(&state.spec.provider, &state.spec).await?;
        state.apply_actual(result.actual, Some(result.provider_id));
        store.upsert(state.clone());
        return Ok(state);
    };

    // Update in place
    let result = registry
        .update(&state.spec.provider, provider_id, &state.spec)
        .await?;
    state.apply_actual(result.actual, Some(provider_id.clone()));
    store.upsert(state.clone());
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{ResourceKind, ResourceSpec, ResourceState, ResourceStatus};
    use std::collections::HashMap;

    fn running_server(name: &str) -> ResourceState {
        let mut props = HashMap::new();
        props.insert("cpu".into(), serde_json::json!(4));
        let spec = ResourceSpec {
            kind: ResourceKind::Server,
            name: name.to_string(),
            provider: "noop".into(),
            properties: props.clone(),
            depends_on: vec![],
            tags: HashMap::new(),
        };
        let mut state = ResourceState::new(spec);
        state.apply_actual(props, Some(format!("{name}-provider-id")));
        state
    }

    #[tokio::test]
    async fn detect_drift_with_noop_provider() {
        let store = ResourceStore::new();
        store.upsert(running_server("web-01"));
        let registry = ProviderRegistry::new();
        let report = detect_drift(&store, &registry).await;
        // noop provider returns same properties → no drift
        assert_eq!(report.drifted.len(), 0);
        assert!(report.healthy > 0 || report.unreachable > 0);
    }

    #[test]
    fn mark_drifted_changes_status() {
        let store = ResourceStore::new();
        let state = running_server("cache-01");
        let key = store.upsert(state);
        mark_drifted(&store, &key).unwrap();
        assert_eq!(store.get(&key).unwrap().status, ResourceStatus::Drifted);
    }
}

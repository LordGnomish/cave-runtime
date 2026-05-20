// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rollback support — revert resource changes using stored history.

use crate::error::{InfraError, InfraResult};
use crate::provider::ProviderRegistry;
use crate::resource::{ResourceState, ResourceStatus, ResourceStore};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackRecord {
    pub id: Uuid,
    pub resource_key: String,
    pub rolled_back_at: DateTime<Utc>,
    pub from_version: u64,
    pub to_version: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// Roll back a single resource to its previous version.
pub async fn rollback_resource(
    store: &ResourceStore,
    registry: &ProviderRegistry,
    key: &str,
) -> InfraResult<RollbackRecord> {
    let current = store.get(key)?;
    let from_version = current.version;

    let prev = store.restore_previous(key)?;
    let to_version = prev.version;

    // If the resource had a provider ID in the previous state, re-apply it
    if let Some(provider_id) = &prev.provider_id {
        let result = registry
            .update(&prev.spec.provider, provider_id, &prev.spec)
            .await
            .map_err(|e| InfraError::RollbackFailed(e.to_string()))?;
        let mut updated = store.get(key)?;
        updated.apply_actual(result.actual, Some(provider_id.clone()));
        store.upsert(updated);
    }

    Ok(RollbackRecord {
        id: Uuid::new_v4(),
        resource_key: key.to_string(),
        rolled_back_at: Utc::now(),
        from_version,
        to_version,
        success: true,
        error: None,
    })
}

/// Roll back multiple resources (typically after a failed plan apply).
pub async fn rollback_batch(
    store: &ResourceStore,
    registry: &ProviderRegistry,
    keys: &[&str],
) -> Vec<RollbackRecord> {
    let mut records = Vec::new();
    // Roll back in reverse order
    for key in keys.iter().rev() {
        let record = match rollback_resource(store, registry, key).await {
            Ok(r) => r,
            Err(e) => RollbackRecord {
                id: Uuid::new_v4(),
                resource_key: key.to_string(),
                rolled_back_at: Utc::now(),
                from_version: 0,
                to_version: 0,
                success: false,
                error: Some(e.to_string()),
            },
        };
        records.push(record);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{ResourceKind, ResourceSpec};
    use std::collections::HashMap;

    fn make_resource(name: &str, cpu: i64) -> ResourceState {
        let mut props = HashMap::new();
        props.insert("cpu".into(), serde_json::json!(cpu));
        let spec = ResourceSpec {
            kind: ResourceKind::Server,
            name: name.to_string(),
            provider: "noop".into(),
            properties: props.clone(),
            depends_on: vec![],
            tags: HashMap::new(),
        };
        let mut state = ResourceState::new(spec);
        state.apply_actual(props, Some(format!("{name}-id")));
        state
    }

    #[tokio::test]
    async fn rollback_to_previous_version() {
        let store = ResourceStore::new();
        let registry = ProviderRegistry::new();

        let v1 = make_resource("srv-01", 4);
        let key = store.upsert(v1);

        // Update to v2
        let mut v2 = make_resource("srv-01", 8);
        v2.spec
            .properties
            .insert("cpu".into(), serde_json::json!(8));
        store.upsert(v2);

        // Rollback
        let record = rollback_resource(&store, &registry, &key).await.unwrap();
        assert!(record.success);

        let current = store.get(&key).unwrap();
        // After rollback, should be back to previous state
        assert_eq!(current.spec.properties["cpu"], serde_json::json!(4));
    }

    #[tokio::test]
    async fn rollback_with_no_history_fails() {
        let store = ResourceStore::new();
        let registry = ProviderRegistry::new();
        let state = make_resource("fresh", 2);
        let key = store.upsert(state);
        // No previous history → rollback should fail
        let result = rollback_resource(&store, &registry, &key).await;
        assert!(result.is_err());
    }
}

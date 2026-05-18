// SPDX-License-Identifier: AGPL-3.0-or-later
//! CatalogStore trait + MemoryCatalogStore + PostgresCatalogStore.
//!
//! Upstream: backstage/plugins/catalog-backend/src/database/DefaultCatalogDatabase.ts

use async_trait::async_trait;
use std::sync::Arc;

use cave_db::persistence::{Storage, StorageExt};

use super::models::{Entity, EntityFilter, Location, RefreshStateRecord};

// ── Error ──────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum CatalogStoreError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<cave_db::persistence::StorageError> for CatalogStoreError {
    fn from(e: cave_db::persistence::StorageError) -> Self {
        CatalogStoreError::Database(e.to_string())
    }
}

// ── Collection names ───────────────────────────────────────────────────────────

const COL_ENTITIES: &str = "entities";
const COL_LOCATIONS: &str = "locations";
const COL_REFRESH_STATE: &str = "refresh_state";

// ── CatalogStore trait ─────────────────────────────────────────────────────────

/// Interface for catalog persistence.
/// Upstream: `ICatalogDatabase` in DefaultCatalogDatabase.ts
#[async_trait]
pub trait CatalogStore: Send + Sync {
    /// Add new entities (upsert by namespace/kind/name).
    /// Upstream: addEntities(txOrKnex, entities)
    async fn add_entities(&self, entities: Vec<Entity>) -> Result<(), CatalogStoreError>;

    /// Update existing entities (same upsert).
    /// Upstream: updateEntities(txOrKnex, entities)
    async fn update_entities(&self, entities: Vec<Entity>) -> Result<(), CatalogStoreError>;

    /// Delete entities by uid.
    /// Upstream: deleteEntities(txOrKnex, uids)
    async fn delete_entities(&self, uids: Vec<String>) -> Result<(), CatalogStoreError>;

    /// Search entities by filter (kind/namespace/name/labels).
    /// Upstream: entities({ filter })
    async fn entities_search(
        &self,
        filter: &EntityFilter,
    ) -> Result<Vec<Entity>, CatalogStoreError>;

    /// Get a single entity by namespace/kind/name.
    async fn entity_by_ref(
        &self,
        namespace: &str,
        kind: &str,
        name: &str,
    ) -> Result<Option<Entity>, CatalogStoreError>;

    /// Insert a new location.
    /// Upstream: insertLocation(txOrKnex, location) — returns assigned id.
    async fn insert_location(&self, location: Location) -> Result<String, CatalogStoreError>;

    /// Delete a location by id. Returns `true` when removed.
    async fn delete_location(&self, id: &str) -> Result<bool, CatalogStoreError>;

    /// List all locations.
    /// Upstream: locations(txOrKnex)
    async fn locations(&self) -> Result<Vec<Location>, CatalogStoreError>;

    /// Upsert refresh state for entity.
    /// Upstream: updateEntityCache(txOrKnex, ...)
    async fn upsert_refresh_state(
        &self,
        state: RefreshStateRecord,
    ) -> Result<(), CatalogStoreError>;

    /// Get refresh state for entity_ref.
    async fn get_refresh_state(
        &self,
        entity_ref: &str,
    ) -> Result<Option<RefreshStateRecord>, CatalogStoreError>;
}

// ── MemoryCatalogStore ─────────────────────────────────────────────────────────

/// In-memory implementation for unit tests and ephemeral dev sessions.
/// Backed by `MemoryStorage` from `cave-db`.
pub struct MemoryCatalogStore {
    storage: Arc<dyn Storage>,
}

impl MemoryCatalogStore {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(cave_db::persistence::MemoryStorage::new()),
        }
    }

    /// Entity id is `"{namespace}/{kind}/{name}"` (all lower-cased for normalisation).
    fn entity_id(entity: &Entity) -> String {
        format!(
            "{}/{}/{}",
            entity.metadata.namespace.to_lowercase(),
            entity.kind.to_lowercase(),
            entity.metadata.name.to_lowercase(),
        )
    }
}

impl Default for MemoryCatalogStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Case-insensitive substring filter helper.
fn ci_eq(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Return true if `entity` matches all non-None fields in `filter`.
fn entity_matches(entity: &Entity, filter: &EntityFilter) -> bool {
    if let Some(ref kind) = filter.kind {
        if !ci_eq(&entity.kind, kind) {
            return false;
        }
    }
    if let Some(ref ns) = filter.namespace {
        if !ci_eq(&entity.metadata.namespace, ns) {
            return false;
        }
    }
    if let Some(ref name) = filter.name {
        if !ci_eq(&entity.metadata.name, name) {
            return false;
        }
    }
    // All specified label key=value pairs must be present and equal.
    for (k, v) in &filter.labels {
        match entity.metadata.labels.get(k) {
            Some(ev) if ev == v => {}
            _ => return false,
        }
    }
    true
}

#[async_trait]
impl CatalogStore for MemoryCatalogStore {
    async fn add_entities(&self, entities: Vec<Entity>) -> Result<(), CatalogStoreError> {
        for entity in entities {
            let id = Self::entity_id(&entity);
            self.storage
                .put(COL_ENTITIES, &id, &entity)
                .await?;
        }
        Ok(())
    }

    async fn update_entities(&self, entities: Vec<Entity>) -> Result<(), CatalogStoreError> {
        // Upsert — same as add.
        self.add_entities(entities).await
    }

    async fn delete_entities(&self, uids: Vec<String>) -> Result<(), CatalogStoreError> {
        // Retrieve all, find those with matching uids, remove by composite key.
        let all: Vec<Entity> = self.storage.list(COL_ENTITIES).await?;
        for entity in all {
            if uids.contains(&entity.metadata.uid) {
                let id = Self::entity_id(&entity);
                self.storage.delete(COL_ENTITIES, &id).await?;
            }
        }
        Ok(())
    }

    async fn entities_search(
        &self,
        filter: &EntityFilter,
    ) -> Result<Vec<Entity>, CatalogStoreError> {
        let all: Vec<Entity> = self.storage.list(COL_ENTITIES).await?;
        Ok(all.into_iter().filter(|e| entity_matches(e, filter)).collect())
    }

    async fn entity_by_ref(
        &self,
        namespace: &str,
        kind: &str,
        name: &str,
    ) -> Result<Option<Entity>, CatalogStoreError> {
        let id = format!(
            "{}/{}/{}",
            namespace.to_lowercase(),
            kind.to_lowercase(),
            name.to_lowercase(),
        );
        let entity: Option<Entity> = self.storage.get(COL_ENTITIES, &id).await?;
        Ok(entity)
    }

    async fn insert_location(&self, location: Location) -> Result<String, CatalogStoreError> {
        let id = location.id.clone();
        self.storage.put(COL_LOCATIONS, &id, &location).await?;
        Ok(id)
    }

    async fn delete_location(&self, id: &str) -> Result<bool, CatalogStoreError> {
        let removed = self.storage.delete(COL_LOCATIONS, id).await?;
        Ok(removed)
    }

    async fn locations(&self) -> Result<Vec<Location>, CatalogStoreError> {
        let locs: Vec<Location> = self.storage.list(COL_LOCATIONS).await?;
        Ok(locs)
    }

    async fn upsert_refresh_state(
        &self,
        state: RefreshStateRecord,
    ) -> Result<(), CatalogStoreError> {
        let key = state.entity_ref.clone();
        self.storage.put(COL_REFRESH_STATE, &key, &state).await?;
        Ok(())
    }

    async fn get_refresh_state(
        &self,
        entity_ref: &str,
    ) -> Result<Option<RefreshStateRecord>, CatalogStoreError> {
        let record: Option<RefreshStateRecord> =
            self.storage.get(COL_REFRESH_STATE, entity_ref).await?;
        Ok(record)
    }
}

// ── PostgresCatalogStore ───────────────────────────────────────────────────────

/// PostgreSQL-backed catalog store.
/// Wraps `cave_db::persistence::PostgresStorage` (schema = "portal").
pub struct PostgresCatalogStore {
    storage: Arc<dyn Storage>,
}

impl PostgresCatalogStore {
    /// Create a store backed by the given `PostgresStorage`.
    /// The caller must already have created the storage (i.e., called
    /// `PostgresStorage::new(pool, "portal").await`).
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    fn entity_id(entity: &Entity) -> String {
        format!(
            "{}/{}/{}",
            entity.metadata.namespace.to_lowercase(),
            entity.kind.to_lowercase(),
            entity.metadata.name.to_lowercase(),
        )
    }
}

#[async_trait]
impl CatalogStore for PostgresCatalogStore {
    async fn add_entities(&self, entities: Vec<Entity>) -> Result<(), CatalogStoreError> {
        for entity in entities {
            let id = Self::entity_id(&entity);
            self.storage.put(COL_ENTITIES, &id, &entity).await?;
        }
        Ok(())
    }

    async fn update_entities(&self, entities: Vec<Entity>) -> Result<(), CatalogStoreError> {
        self.add_entities(entities).await
    }

    async fn delete_entities(&self, uids: Vec<String>) -> Result<(), CatalogStoreError> {
        let all: Vec<Entity> = self.storage.list(COL_ENTITIES).await?;
        for entity in all {
            if uids.contains(&entity.metadata.uid) {
                let id = Self::entity_id(&entity);
                self.storage.delete(COL_ENTITIES, &id).await?;
            }
        }
        Ok(())
    }

    async fn entities_search(
        &self,
        filter: &EntityFilter,
    ) -> Result<Vec<Entity>, CatalogStoreError> {
        let all: Vec<Entity> = self.storage.list(COL_ENTITIES).await?;
        Ok(all.into_iter().filter(|e| entity_matches(e, filter)).collect())
    }

    async fn entity_by_ref(
        &self,
        namespace: &str,
        kind: &str,
        name: &str,
    ) -> Result<Option<Entity>, CatalogStoreError> {
        let id = format!(
            "{}/{}/{}",
            namespace.to_lowercase(),
            kind.to_lowercase(),
            name.to_lowercase(),
        );
        let entity: Option<Entity> = self.storage.get(COL_ENTITIES, &id).await?;
        Ok(entity)
    }

    async fn insert_location(&self, location: Location) -> Result<String, CatalogStoreError> {
        let id = location.id.clone();
        self.storage.put(COL_LOCATIONS, &id, &location).await?;
        Ok(id)
    }

    async fn delete_location(&self, id: &str) -> Result<bool, CatalogStoreError> {
        let removed = self.storage.delete(COL_LOCATIONS, id).await?;
        Ok(removed)
    }

    async fn locations(&self) -> Result<Vec<Location>, CatalogStoreError> {
        let locs: Vec<Location> = self.storage.list(COL_LOCATIONS).await?;
        Ok(locs)
    }

    async fn upsert_refresh_state(
        &self,
        state: RefreshStateRecord,
    ) -> Result<(), CatalogStoreError> {
        let key = state.entity_ref.clone();
        self.storage.put(COL_REFRESH_STATE, &key, &state).await?;
        Ok(())
    }

    async fn get_refresh_state(
        &self,
        entity_ref: &str,
    ) -> Result<Option<RefreshStateRecord>, CatalogStoreError> {
        let record: Option<RefreshStateRecord> =
            self.storage.get(COL_REFRESH_STATE, entity_ref).await?;
        Ok(record)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::models::{Entity, EntityMetadata, Location, RefreshStateRecord};
    use std::collections::HashMap;

    fn make_entity(uid: &str, kind: &str, namespace: &str, name: &str) -> Entity {
        Entity {
            api_version: "backstage.io/v1alpha1".to_string(),
            kind: kind.to_string(),
            metadata: EntityMetadata {
                uid: uid.to_string(),
                name: name.to_string(),
                namespace: namespace.to_string(),
                title: None,
                description: None,
                labels: HashMap::new(),
                annotations: HashMap::new(),
                tags: vec![],
                created_at: None,
                updated_at: None,
            },
            spec: None,
            relations: vec![],
        }
    }

    fn make_entity_with_labels(
        uid: &str,
        kind: &str,
        namespace: &str,
        name: &str,
        labels: HashMap<String, String>,
    ) -> Entity {
        let mut e = make_entity(uid, kind, namespace, name);
        e.metadata.labels = labels;
        e
    }

    fn make_store() -> MemoryCatalogStore {
        MemoryCatalogStore::new()
    }

    /// Test 1: add one entity, entities_search() returns it.
    #[tokio::test]
    async fn add_entities_inserts_single() {
        let store = make_store();
        let entity = make_entity("uid-1", "Component", "default", "my-svc");
        store.add_entities(vec![entity]).await.unwrap();

        let results = store
            .entities_search(&EntityFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.uid, "uid-1");
    }

    /// Test 2: add same entity twice (same namespace/kind/name), only one stored.
    #[tokio::test]
    async fn add_entities_upserts_on_conflict() {
        let store = make_store();
        let e1 = make_entity("uid-1", "Component", "default", "my-svc");
        let mut e2 = make_entity("uid-1", "Component", "default", "my-svc");
        e2.spec = Some(serde_json::json!({"type": "service"}));

        store.add_entities(vec![e1]).await.unwrap();
        store.add_entities(vec![e2]).await.unwrap();

        let results = store
            .entities_search(&EntityFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        // Second write wins.
        assert!(results[0].spec.is_some());
    }

    /// Test 3: add entity, update spec, verify changed.
    #[tokio::test]
    async fn update_entities_changes_spec() {
        let store = make_store();
        let entity = make_entity("uid-1", "Component", "default", "my-svc");
        store.add_entities(vec![entity]).await.unwrap();

        let mut updated = make_entity("uid-1", "Component", "default", "my-svc");
        updated.spec = Some(serde_json::json!({"lifecycle": "production"}));
        store.update_entities(vec![updated]).await.unwrap();

        let results = store
            .entities_search(&EntityFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        let spec = results[0].spec.as_ref().unwrap();
        assert_eq!(spec["lifecycle"], "production");
    }

    /// Test 4: add entity, delete by uid, not found afterwards.
    #[tokio::test]
    async fn delete_entities_removes_by_uid() {
        let store = make_store();
        let entity = make_entity("uid-1", "Component", "default", "my-svc");
        store.add_entities(vec![entity]).await.unwrap();
        store
            .delete_entities(vec!["uid-1".to_string()])
            .await
            .unwrap();

        let results = store
            .entities_search(&EntityFilter::default())
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    /// Test 5: filter by kind=Component returns only Component entities.
    #[tokio::test]
    async fn entities_search_by_kind() {
        let store = make_store();
        store
            .add_entities(vec![
                make_entity("uid-1", "Component", "default", "svc-a"),
                make_entity("uid-2", "API", "default", "api-a"),
                make_entity("uid-3", "Component", "default", "svc-b"),
            ])
            .await
            .unwrap();

        let filter = EntityFilter {
            kind: Some("Component".to_string()),
            ..Default::default()
        };
        let results = store.entities_search(&filter).await.unwrap();
        assert_eq!(results.len(), 2);
        for e in &results {
            assert_eq!(e.kind.to_lowercase(), "component");
        }
    }

    /// Test 6: filter by namespace returns only matching entities.
    #[tokio::test]
    async fn entities_search_by_namespace() {
        let store = make_store();
        store
            .add_entities(vec![
                make_entity("uid-1", "Component", "default", "svc-a"),
                make_entity("uid-2", "Component", "production", "svc-b"),
            ])
            .await
            .unwrap();

        let filter = EntityFilter {
            namespace: Some("production".to_string()),
            ..Default::default()
        };
        let results = store.entities_search(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.namespace, "production");
    }

    /// Test 7: filter by name returns exact match (case-insensitive).
    #[tokio::test]
    async fn entities_search_by_name() {
        let store = make_store();
        store
            .add_entities(vec![
                make_entity("uid-1", "Component", "default", "svc-a"),
                make_entity("uid-2", "Component", "default", "svc-b"),
            ])
            .await
            .unwrap();

        let filter = EntityFilter {
            name: Some("SVC-A".to_string()),
            ..Default::default()
        };
        let results = store.entities_search(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.name, "svc-a");
    }

    /// Test 8: filter by label key=value.
    #[tokio::test]
    async fn entities_search_by_label() {
        let store = make_store();
        let mut labels_match = HashMap::new();
        labels_match.insert("env".to_string(), "prod".to_string());
        let mut labels_no_match = HashMap::new();
        labels_no_match.insert("env".to_string(), "staging".to_string());

        store
            .add_entities(vec![
                make_entity_with_labels("uid-1", "Component", "default", "svc-a", labels_match),
                make_entity_with_labels(
                    "uid-2",
                    "Component",
                    "default",
                    "svc-b",
                    labels_no_match,
                ),
            ])
            .await
            .unwrap();

        let filter = EntityFilter {
            labels: {
                let mut m = HashMap::new();
                m.insert("env".to_string(), "prod".to_string());
                m
            },
            ..Default::default()
        };
        let results = store.entities_search(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.uid, "uid-1");
    }

    /// Test 9: empty filter returns all entities.
    #[tokio::test]
    async fn entities_search_all_returns_all() {
        let store = make_store();
        store
            .add_entities(vec![
                make_entity("uid-1", "Component", "default", "svc-a"),
                make_entity("uid-2", "API", "default", "api-a"),
                make_entity("uid-3", "Resource", "ns-x", "db"),
            ])
            .await
            .unwrap();

        let results = store
            .entities_search(&EntityFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
    }

    /// Test 10: entity_by_ref finds the entity.
    #[tokio::test]
    async fn entity_by_ref_found() {
        let store = make_store();
        let entity = make_entity("uid-1", "Component", "default", "my-svc");
        store.add_entities(vec![entity]).await.unwrap();

        let found = store
            .entity_by_ref("default", "Component", "my-svc")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().metadata.uid, "uid-1");
    }

    /// Test 11: entity_by_ref returns None for missing entity.
    #[tokio::test]
    async fn entity_by_ref_not_found() {
        let store = make_store();
        let found = store
            .entity_by_ref("default", "Component", "ghost")
            .await
            .unwrap();
        assert!(found.is_none());
    }

    /// Test 12: insert a location, locations() returns it.
    #[tokio::test]
    async fn insert_location_creates_entry() {
        let store = make_store();
        let loc = Location {
            id: "loc-1".to_string(),
            type_: "url".to_string(),
            target: "https://example.com/catalog-info.yaml".to_string(),
            presence: Some("required".to_string()),
        };
        let returned_id = store.insert_location(loc).await.unwrap();
        assert_eq!(returned_id, "loc-1");

        let locs = store.locations().await.unwrap();
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].id, "loc-1");
    }

    /// Test 13: insert then delete location, locations() empty.
    #[tokio::test]
    async fn delete_location_removes_entry() {
        let store = make_store();
        let loc = Location {
            id: "loc-1".to_string(),
            type_: "url".to_string(),
            target: "https://example.com/catalog-info.yaml".to_string(),
            presence: None,
        };
        store.insert_location(loc).await.unwrap();
        let removed = store.delete_location("loc-1").await.unwrap();
        assert!(removed);

        let locs = store.locations().await.unwrap();
        assert!(locs.is_empty());
    }

    /// Test 14: upsert refresh state, then get returns the record.
    #[tokio::test]
    async fn upsert_refresh_state_creates() {
        let store = make_store();
        let state = RefreshStateRecord {
            entity_ref: "component:default/my-svc".to_string(),
            unprocessed_entity: serde_json::json!({"kind": "Component"}),
            errors: None,
            next_update_at: None,
            last_discovery_at: None,
        };
        store.upsert_refresh_state(state).await.unwrap();

        let found = store
            .get_refresh_state("component:default/my-svc")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(
            found.unwrap().entity_ref,
            "component:default/my-svc"
        );
    }

    /// Test 15: upsert same ref twice, second write wins.
    #[tokio::test]
    async fn upsert_refresh_state_updates() {
        let store = make_store();
        let state1 = RefreshStateRecord {
            entity_ref: "component:default/my-svc".to_string(),
            unprocessed_entity: serde_json::json!({"kind": "Component", "v": 1}),
            errors: None,
            next_update_at: None,
            last_discovery_at: None,
        };
        let state2 = RefreshStateRecord {
            entity_ref: "component:default/my-svc".to_string(),
            unprocessed_entity: serde_json::json!({"kind": "Component", "v": 2}),
            errors: Some("parse warning".to_string()),
            next_update_at: None,
            last_discovery_at: None,
        };
        store.upsert_refresh_state(state1).await.unwrap();
        store.upsert_refresh_state(state2).await.unwrap();

        let found = store
            .get_refresh_state("component:default/my-svc")
            .await
            .unwrap()
            .unwrap();
        // Second write should win.
        assert_eq!(found.unprocessed_entity["v"], 2);
        assert_eq!(found.errors, Some("parse warning".to_string()));
    }
}

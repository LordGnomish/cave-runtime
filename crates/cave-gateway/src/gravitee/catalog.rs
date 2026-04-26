//! API catalog + versioning (Gravitee API Registry).
//!
//! Manages API definitions, versions, publication states, deprecation.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionState {
    Draft,
    Published,
    Deprecated,
    Retired,
}

/// API version within a catalog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiVersion {
    pub version: String,
    pub openapi_url: Option<String>,
    pub base_path: String,
    pub upstream_url: String,
    pub state: VersionState,
    pub published_at: Option<DateTime<Utc>>,
    pub deprecated: bool,
    pub sunset_at: Option<DateTime<Utc>>,
}

/// Catalog entry representing a managed API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub owner_team: String,
    pub category: String,
    pub versions: Vec<ApiVersion>,
    pub tags: Vec<String>,
    pub published: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CatalogEntry {
    /// Create a new catalog entry.
    pub fn new(name: String, owner_team: String, category: String) -> Self {
        let slug = name.to_lowercase().replace(' ', "-");
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            slug,
            description: String::new(),
            owner_team,
            category,
            versions: Vec::new(),
            tags: Vec::new(),
            published: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Add a new version to this entry.
    pub fn add_version(&mut self, version: ApiVersion) {
        self.versions.push(version);
        self.updated_at = Utc::now();
    }
}

/// API catalog store.
pub struct CatalogStore {
    entries: DashMap<Uuid, CatalogEntry>,
    by_slug: DashMap<String, Uuid>,
}

impl CatalogStore {
    /// Create a new catalog store.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: DashMap::new(),
            by_slug: DashMap::new(),
        })
    }

    /// Create or update a catalog entry.
    pub fn upsert(&self, entry: CatalogEntry) {
        self.by_slug.insert(entry.slug.clone(), entry.id);
        self.entries.insert(entry.id, entry);
    }

    /// Get an entry by ID.
    pub fn get(&self, id: Uuid) -> Option<CatalogEntry> {
        self.entries.get(&id).map(|e| e.value().clone())
    }

    /// Get an entry by slug.
    pub fn get_by_slug(&self, slug: &str) -> Option<CatalogEntry> {
        let id = self.by_slug.get(slug)?;
        self.entries.get(id.value()).map(|e| e.value().clone())
    }

    /// List all entries.
    pub fn list(&self) -> Vec<CatalogEntry> {
        self.entries.iter().map(|e| e.value().clone()).collect()
    }

    /// Delete an entry by ID.
    pub fn delete(&self, id: Uuid) -> bool {
        if let Some((_, entry)) = self.entries.remove(&id) {
            self.by_slug.remove(&entry.slug);
            return true;
        }
        false
    }

    /// Publish a specific version.
    pub fn publish(&self, id: Uuid, version: &str) -> bool {
        if let Some(mut entry) = self.entries.get_mut(&id) {
            if let Some(v) = entry.versions.iter_mut().find(|v| v.version == version) {
                v.state = VersionState::Published;
                v.published_at = Some(Utc::now());
                entry.published = true;
                return true;
            }
        }
        false
    }

    /// Deprecate a version.
    pub fn deprecate(&self, id: Uuid, version: &str, sunset_at: DateTime<Utc>) -> bool {
        if let Some(mut entry) = self.entries.get_mut(&id) {
            if let Some(v) = entry.versions.iter_mut().find(|v| v.version == version) {
                v.state = VersionState::Deprecated;
                v.deprecated = true;
                v.sunset_at = Some(sunset_at);
                entry.updated_at = Utc::now();
                return true;
            }
        }
        false
    }

    /// Retire a version.
    pub fn retire(&self, id: Uuid, version: &str) -> bool {
        if let Some(mut entry) = self.entries.get_mut(&id) {
            if let Some(v) = entry.versions.iter_mut().find(|v| v.version == version) {
                v.state = VersionState::Retired;
                entry.updated_at = Utc::now();
                return true;
            }
        }
        false
    }

    /// Search catalog by query string (name or description).
    pub fn search(&self, query: &str) -> Vec<CatalogEntry> {
        let q_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                let entry = e.value();
                entry.name.to_lowercase().contains(&q_lower)
                    || entry.description.to_lowercase().contains(&q_lower)
            })
            .map(|e| e.value().clone())
            .collect()
    }

    /// List entries by category.
    pub fn list_by_category(&self, category: &str) -> Vec<CatalogEntry> {
        self.entries
            .iter()
            .filter(|e| e.value().category == category)
            .map(|e| e.value().clone())
            .collect()
    }
}

impl Default for CatalogStore {
    fn default() -> Self {
        CatalogStore {
            entries: DashMap::new(),
            by_slug: DashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_create_and_get() {
        let store = CatalogStore::new();
        let mut entry = CatalogEntry::new("UserAPI".to_string(), "team-a".to_string(), "core".to_string());
        let id = entry.id;

        store.upsert(entry.clone());
        let retrieved = store.get(id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "UserAPI");
    }

    #[test]
    fn test_catalog_get_by_slug() {
        let store = CatalogStore::new();
        let entry = CatalogEntry::new("User API".to_string(), "team-a".to_string(), "core".to_string());
        store.upsert(entry.clone());

        let retrieved = store.get_by_slug(&entry.slug);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "User API");
    }

    #[test]
    fn test_catalog_publish_version() {
        let store = CatalogStore::new();
        let mut entry = CatalogEntry::new("UserAPI".to_string(), "team-a".to_string(), "core".to_string());
        let id = entry.id;
        entry.add_version(ApiVersion {
            version: "1.0.0".to_string(),
            openapi_url: Some("https://example.com/openapi.json".to_string()),
            base_path: "/v1".to_string(),
            upstream_url: "http://backend:3000".to_string(),
            state: VersionState::Draft,
            published_at: None,
            deprecated: false,
            sunset_at: None,
        });

        store.upsert(entry);
        assert!(store.publish(id, "1.0.0"));

        let retrieved = store.get(id).unwrap();
        let v = retrieved.versions.iter().find(|v| v.version == "1.0.0").unwrap();
        assert_eq!(v.state, VersionState::Published);
        assert!(v.published_at.is_some());
    }

    #[test]
    fn test_catalog_search() {
        let store = CatalogStore::new();
        let mut e1 = CatalogEntry::new("UserAPI".to_string(), "team-a".to_string(), "core".to_string());
        e1.description = "User management API".to_string();
        let mut e2 = CatalogEntry::new("OrderAPI".to_string(), "team-b".to_string(), "sales".to_string());
        e2.description = "Order processing system".to_string();

        store.upsert(e1);
        store.upsert(e2);

        let results = store.search("User");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "UserAPI");
    }

    #[test]
    fn test_catalog_list_by_category() {
        let store = CatalogStore::new();
        let e1 = CatalogEntry::new("UserAPI".to_string(), "team-a".to_string(), "core".to_string());
        let e2 = CatalogEntry::new("OrderAPI".to_string(), "team-b".to_string(), "core".to_string());
        let e3 = CatalogEntry::new("ReportAPI".to_string(), "team-c".to_string(), "analytics".to_string());

        store.upsert(e1);
        store.upsert(e2);
        store.upsert(e3);

        let core_apis = store.list_by_category("core");
        assert_eq!(core_apis.len(), 2);
        let analytics_apis = store.list_by_category("analytics");
        assert_eq!(analytics_apis.len(), 1);
    }
}

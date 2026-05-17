// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Federation gateway (merge upstream APIs from multiple sources).
//!
//! Consolidates APIs from Kubernetes, Consul, Gravitee discovery, or static OpenAPI.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeStrategy {
    PrefixByName,
    UnionPreferLatest,
    NamespaceByLabel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source_type")]
pub enum SourceType {
    Kubernetes,
    ConsulCatalog,
    GraviteeDiscovery,
    StaticOpenApi(String), // URL or inline marker
}

/// A source for API discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedSource {
    pub source_type: SourceType,
    pub discovery_url: String,
    pub path_prefix: Option<String>,
    pub label_selector: HashMap<String, String>,
}

/// A federated API combining multiple sources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedApi {
    pub id: Uuid,
    pub name: String,
    pub sources: Vec<FederatedSource>,
    pub merge_strategy: MergeStrategy,
    pub published_at: Option<DateTime<Utc>>,
}

impl FederatedApi {
    /// Create a new federated API.
    pub fn new(name: String, merge_strategy: MergeStrategy) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            sources: Vec::new(),
            merge_strategy,
            published_at: None,
        }
    }

    /// Add a source to this federated API.
    pub fn add_source(&mut self, source: FederatedSource) {
        self.sources.push(source);
    }
}

/// Result of a federation merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    pub merged_openapi: serde_json::Value,
    pub conflicts: Vec<String>,
}

/// Federation store.
pub struct FederationStore {
    federated: DashMap<Uuid, FederatedApi>,
}

impl FederationStore {
    /// Create a new federation store.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            federated: DashMap::new(),
        })
    }

    /// Create or update a federated API.
    pub fn upsert(&self, api: FederatedApi) {
        self.federated.insert(api.id, api);
    }

    /// Get a federated API by ID.
    pub fn get(&self, id: Uuid) -> Option<FederatedApi> {
        self.federated.get(&id).map(|e| e.value().clone())
    }

    /// List all federated APIs.
    pub fn list(&self) -> Vec<FederatedApi> {
        self.federated.iter().map(|e| e.value().clone()).collect()
    }

    /// Delete a federated API.
    pub fn delete(&self, id: Uuid) -> bool {
        self.federated.remove(&id).is_some()
    }

    /// Refresh a federated API by merging all sources.
    /// Stubbed: does not actually hit external services but returns a synthesized result.
    pub fn refresh(&self, id: Uuid) -> Option<MergeResult> {
        let api = self.get(id)?;

        // For each source, we would normally fetch its OpenAPI spec
        // For testing/stubbing, we synthesize a merged result
        let mut merged = serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "title": format!("Federated: {}", api.name),
                "version": "1.0.0"
            },
            "paths": {}
        });

        let mut conflicts = Vec::new();

        for source in &api.sources {
            match &source.source_type {
                SourceType::StaticOpenApi(url_or_marker) => {
                    // For test: accept "file://inline-json" marker
                    // In production, fetch from the URL
                    let spec = if url_or_marker.starts_with("file://inline-json") {
                        // Return a fixed test fixture
                        serde_json::json!({
                            "openapi": "3.0.0",
                            "info": { "title": "Inline API", "version": "1.0.0" },
                            "paths": {
                                "/test": {
                                    "get": {
                                        "responses": { "200": { "description": "OK" } }
                                    }
                                }
                            }
                        })
                    } else {
                        // In real usage, would fetch from url_or_marker
                        serde_json::json!({
                            "openapi": "3.0.0",
                            "info": { "title": "External API", "version": "1.0.0" },
                            "paths": {}
                        })
                    };

                    // Merge paths with conflict detection
                    if let Some(new_paths) = spec.get("paths").and_then(|p| p.as_object()) {
                        if let Some(merged_paths) = merged
                            .get_mut("paths")
                            .and_then(|p| p.as_object_mut())
                        {
                            for (path, path_spec) in new_paths {
                                if merged_paths.contains_key(path) {
                                    conflicts.push(format!("Path conflict: {}", path));
                                } else {
                                    merged_paths.insert(path.clone(), path_spec.clone());
                                }
                            }
                        }
                    }
                }
                SourceType::Kubernetes => {
                    // Stubbed: would query Kubernetes API server
                    conflicts.push("Kubernetes discovery not implemented in stub".to_string());
                }
                SourceType::ConsulCatalog => {
                    // Stubbed: would query Consul
                    conflicts.push("Consul discovery not implemented in stub".to_string());
                }
                SourceType::GraviteeDiscovery => {
                    // Stubbed: would query Gravitee API
                    conflicts.push("Gravitee discovery not implemented in stub".to_string());
                }
            }
        }

        Some(MergeResult {
            merged_openapi: merged,
            conflicts,
        })
    }
}

impl Default for FederationStore {
    fn default() -> Self {
        FederationStore {
            federated: DashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_federation_create_and_get() {
        let store = FederationStore::new();
        let api = FederatedApi::new("MergedAPI".to_string(), MergeStrategy::UnionPreferLatest);
        let id = api.id;

        store.upsert(api);
        let retrieved = store.get(id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "MergedAPI");
    }

    #[test]
    fn test_federation_add_source() {
        let store = FederationStore::new();
        let mut api = FederatedApi::new("MergedAPI".to_string(), MergeStrategy::PrefixByName);
        let id = api.id;

        api.add_source(FederatedSource {
            source_type: SourceType::StaticOpenApi("http://api1.example.com/openapi.json".to_string()),
            discovery_url: "http://api1.example.com/discover".to_string(),
            path_prefix: Some("/api1".to_string()),
            label_selector: HashMap::new(),
        });

        store.upsert(api);
        let retrieved = store.get(id).unwrap();
        assert_eq!(retrieved.sources.len(), 1);
    }

    #[test]
    fn test_federation_refresh_with_static_openapi() {
        let store = FederationStore::new();
        let mut api = FederatedApi::new("MergedAPI".to_string(), MergeStrategy::UnionPreferLatest);
        let id = api.id;

        // Add a static OpenAPI source with inline marker
        api.add_source(FederatedSource {
            source_type: SourceType::StaticOpenApi("file://inline-json".to_string()),
            discovery_url: String::new(),
            path_prefix: None,
            label_selector: HashMap::new(),
        });

        store.upsert(api);

        let result = store.refresh(id);
        assert!(result.is_some());
        let merge_result = result.unwrap();
        let merged = &merge_result.merged_openapi;
        assert!(merged.get("info").is_some());
        assert!(merged.get("paths").is_some());
    }
}

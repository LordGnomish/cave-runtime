//! cave-search — sovereign full-text search engine.
//!
//! Replaces OpenSearch / Elasticsearch with a built-in inverted index and
//! BM25 scoring, with adapter stubs for external backends (OpenSearch, Azure
//! AI Search) when sovereign deployment is not required.
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────┐
//! │  REST Layer  (Axum, OpenSearch-wire-compatible)        │
//! ├───────────────────────────────────────────────────────┤
//! │  SearchEngine trait                                    │
//! │  ├─ BuiltinSearchEngine (inverted index + BM25)        │
//! │  ├─ OpenSearchAdapter   (pass-through to OpenSearch)   │
//! │  └─ AzureSearchAdapter  (Azure AI Search)              │
//! ├───────────────────────────────────────────────────────┤
//! │  Query DSL executor  (bool, match, range, term, …)    │
//! ├───────────────────────────────────────────────────────┤
//! │  Text analysis pipeline  (tokenise → filter → stem)   │
//! └───────────────────────────────────────────────────────┘
//! ```

pub mod engine;
pub mod index;
pub mod models;
pub mod query;
pub mod routes;

use async_trait::async_trait;
use axum::Router;
use std::sync::Arc;
use thiserror::Error;

use engine::BuiltinSearchEngine;
use models::{
    BulkResponse, Document, IndexMapping, IndexSettings, SearchRequest, SearchResponse,
};

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("index not found: {0}")]
    IndexNotFound(String),

    #[error("index already exists: {0}")]
    IndexAlreadyExists(String),

    #[error("document not found: {0}/{1}")]
    DocumentNotFound(String, String),

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("invalid mapping: {0}")]
    InvalidMapping(String),

    #[error("tenant not allowed to access index: {0}")]
    TenantAccessDenied(String),

    #[error("backend error: {0}")]
    BackendError(String),

    #[error("serialisation error: {0}")]
    Serialisation(#[from] serde_json::Error),
}

impl axum::response::IntoResponse for SearchError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_type) = match &self {
            SearchError::IndexNotFound(_) => {
                (axum::http::StatusCode::NOT_FOUND, "index_not_found_exception")
            }
            SearchError::IndexAlreadyExists(_) => {
                (axum::http::StatusCode::BAD_REQUEST, "resource_already_exists_exception")
            }
            SearchError::DocumentNotFound(_, _) => {
                (axum::http::StatusCode::NOT_FOUND, "not_found")
            }
            SearchError::InvalidQuery(_) => {
                (axum::http::StatusCode::BAD_REQUEST, "query_parse_exception")
            }
            SearchError::TenantAccessDenied(_) => {
                (axum::http::StatusCode::FORBIDDEN, "security_exception")
            }
            _ => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };

        (
            status,
            axum::Json(serde_json::json!({
                "error": {
                    "type": error_type,
                    "reason": self.to_string()
                },
                "status": status.as_u16()
            })),
        )
            .into_response()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SearchEngine trait
// ─────────────────────────────────────────────────────────────────────────────

/// The core search engine contract.
///
/// The built-in implementation uses an in-memory inverted index with BM25
/// scoring.  External adapters (OpenSearch, Azure AI Search) implement the
/// same trait so the rest of the platform can swap backends via configuration
/// without modifying business logic.
#[async_trait]
pub trait SearchEngine: Send + Sync {
    /// Create a new index with the given mapping and settings.
    async fn create_index(
        &self,
        name: &str,
        mapping: IndexMapping,
        settings: IndexSettings,
    ) -> Result<(), SearchError>;

    /// Permanently delete an index and all its documents.
    async fn delete_index(&self, name: &str) -> Result<(), SearchError>;

    /// Index a single document, returning the document ID.
    async fn index_document(
        &self,
        index: &str,
        doc: Document,
    ) -> Result<String, SearchError>;

    /// Retrieve a single document by ID.
    async fn get_document(
        &self,
        index: &str,
        id: &str,
    ) -> Result<Option<Document>, SearchError>;

    /// Delete a single document by ID.  Returns `false` if not found.
    async fn delete_document(
        &self,
        index: &str,
        id: &str,
    ) -> Result<bool, SearchError>;

    /// Execute a search request and return ranked results.
    async fn search(
        &self,
        index: &str,
        request: SearchRequest,
    ) -> Result<SearchResponse, SearchError>;

    /// Return the number of documents in the index.
    async fn count(&self, index: &str) -> Result<u64, SearchError>;

    /// Index multiple documents in a single operation.
    async fn bulk_index(
        &self,
        index: &str,
        docs: Vec<Document>,
    ) -> Result<BulkResponse, SearchError>;

    /// Check whether an index exists.
    async fn index_exists(&self, name: &str) -> bool;

    /// List all known index names.
    async fn list_indices(&self) -> Vec<String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// External adapter stubs
// ─────────────────────────────────────────────────────────────────────────────

/// Adapter that proxies requests to an external OpenSearch cluster.
///
/// This stub returns `BackendError` for every call.  Wire up a real
/// `reqwest::Client` and the OpenSearch REST API to enable it.
pub struct OpenSearchAdapter {
    pub endpoint: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls_verify: bool,
}

impl OpenSearchAdapter {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            username: None,
            password: None,
            tls_verify: true,
        }
    }
}

#[async_trait]
impl SearchEngine for OpenSearchAdapter {
    async fn create_index(&self, name: &str, _mapping: IndexMapping, _settings: IndexSettings) -> Result<(), SearchError> {
        Err(SearchError::BackendError(format!("OpenSearchAdapter: create_index({}) not implemented — connect to {}", name, self.endpoint)))
    }
    async fn delete_index(&self, name: &str) -> Result<(), SearchError> {
        Err(SearchError::BackendError(format!("OpenSearchAdapter: delete_index({}) not implemented", name)))
    }
    async fn index_document(&self, _index: &str, doc: Document) -> Result<String, SearchError> {
        Err(SearchError::BackendError("OpenSearchAdapter: index_document not implemented".into()))
    }
    async fn get_document(&self, index: &str, id: &str) -> Result<Option<Document>, SearchError> {
        Err(SearchError::BackendError(format!("OpenSearchAdapter: get_document({}/{}) not implemented", index, id)))
    }
    async fn delete_document(&self, index: &str, id: &str) -> Result<bool, SearchError> {
        Err(SearchError::BackendError(format!("OpenSearchAdapter: delete_document({}/{}) not implemented", index, id)))
    }
    async fn search(&self, index: &str, _request: SearchRequest) -> Result<SearchResponse, SearchError> {
        Err(SearchError::BackendError(format!("OpenSearchAdapter: search({}) not implemented", index)))
    }
    async fn count(&self, index: &str) -> Result<u64, SearchError> {
        Err(SearchError::BackendError(format!("OpenSearchAdapter: count({}) not implemented", index)))
    }
    async fn bulk_index(&self, index: &str, _docs: Vec<Document>) -> Result<BulkResponse, SearchError> {
        Err(SearchError::BackendError(format!("OpenSearchAdapter: bulk_index({}) not implemented", index)))
    }
    async fn index_exists(&self, _name: &str) -> bool { false }
    async fn list_indices(&self) -> Vec<String> { vec![] }
}

/// Adapter that proxies requests to Azure AI Search.
pub struct AzureSearchAdapter {
    pub service_name: String,
    pub api_key: String,
    pub api_version: String,
}

impl AzureSearchAdapter {
    pub fn new(service_name: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            api_key: api_key.into(),
            api_version: "2023-11-01".into(),
        }
    }
}

#[async_trait]
impl SearchEngine for AzureSearchAdapter {
    async fn create_index(&self, name: &str, _mapping: IndexMapping, _settings: IndexSettings) -> Result<(), SearchError> {
        Err(SearchError::BackendError(format!("AzureSearchAdapter: create_index({}) not implemented — connect to {}.search.windows.net", name, self.service_name)))
    }
    async fn delete_index(&self, name: &str) -> Result<(), SearchError> {
        Err(SearchError::BackendError(format!("AzureSearchAdapter: delete_index({}) not implemented", name)))
    }
    async fn index_document(&self, _index: &str, _doc: Document) -> Result<String, SearchError> {
        Err(SearchError::BackendError("AzureSearchAdapter: index_document not implemented".into()))
    }
    async fn get_document(&self, index: &str, id: &str) -> Result<Option<Document>, SearchError> {
        Err(SearchError::BackendError(format!("AzureSearchAdapter: get_document({}/{}) not implemented", index, id)))
    }
    async fn delete_document(&self, index: &str, id: &str) -> Result<bool, SearchError> {
        Err(SearchError::BackendError(format!("AzureSearchAdapter: delete_document({}/{}) not implemented", index, id)))
    }
    async fn search(&self, index: &str, _request: SearchRequest) -> Result<SearchResponse, SearchError> {
        Err(SearchError::BackendError(format!("AzureSearchAdapter: search({}) not implemented", index)))
    }
    async fn count(&self, index: &str) -> Result<u64, SearchError> {
        Err(SearchError::BackendError(format!("AzureSearchAdapter: count({}) not implemented", index)))
    }
    async fn bulk_index(&self, index: &str, _docs: Vec<Document>) -> Result<BulkResponse, SearchError> {
        Err(SearchError::BackendError(format!("AzureSearchAdapter: bulk_index({}) not implemented", index)))
    }
    async fn index_exists(&self, _name: &str) -> bool { false }
    async fn list_indices(&self) -> Vec<String> { vec![] }
}

// ─────────────────────────────────────────────────────────────────────────────
// State
// ─────────────────────────────────────────────────────────────────────────────

/// Module state passed through the Axum router.
pub struct SearchState {
    /// The active search engine (built-in by default).
    pub engine: Arc<BuiltinSearchEngine>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self { engine: BuiltinSearchEngine::new() }
    }
}

impl SearchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Use an external OpenSearch cluster instead of the built-in engine.
    ///
    /// Note: the adapter stubs are not yet wired — call this once you have
    /// connected a live `reqwest::Client`.
    pub fn with_opensearch(endpoint: impl Into<String>) -> Self {
        // For now return the default built-in engine.  Replace with an
        // `Arc<OpenSearchAdapter>` once the adapter is fully implemented.
        tracing::warn!(
            endpoint = endpoint.into().as_str(),
            "OpenSearchAdapter stub selected — falling back to built-in engine"
        );
        Self::default()
    }
}

/// Module identifier consumed by the portal / health aggregation.
pub const MODULE_NAME: &str = "cave-search";

/// Upstream project this module replaces.
pub const UPSTREAM: &str = "OpenSearch";

// ─────────────────────────────────────────────────────────────────────────────
// Public router factory
// ─────────────────────────────────────────────────────────────────────────────

pub fn router(state: Arc<SearchState>) -> Router {
    routes::create_router(state)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn search_state_default_creates_engine() {
        let state = SearchState::default();
        assert!(state.engine.list_indices().is_empty());
    }

    #[test]
    fn error_into_response_not_found() {
        let err = SearchError::IndexNotFound("my-index".into());
        assert!(err.to_string().contains("my-index"));
    }

    #[test]
    fn error_into_response_already_exists() {
        let err = SearchError::IndexAlreadyExists("test".into());
        assert!(err.to_string().contains("test"));
    }

    #[test]
    fn opensearch_adapter_endpoint_stored() {
        let adapter = OpenSearchAdapter::new("https://search.example.com:9200");
        assert!(adapter.endpoint.contains("9200"));
        assert!(adapter.tls_verify);
    }

    #[test]
    fn azure_adapter_api_version_set() {
        let adapter = AzureSearchAdapter::new("my-service", "secret-key");
        assert_eq!(adapter.api_version, "2023-11-01");
        assert_eq!(adapter.service_name, "my-service");
    }

    #[test]
    fn engine_create_and_search_full_flow() {
        let state = SearchState::default();
        let mapping = models::IndexMapping::default();
        let settings = models::IndexSettings::default();

        state.engine.create_index("articles", mapping, settings, HashMap::new()).unwrap();

        let mut source = HashMap::new();
        source.insert("title".to_string(), json!("Introduction to Rust"));
        source.insert("body".to_string(), json!("Rust is a systems programming language."));
        let doc = Document::new("articles", source);
        state.engine.index_document("articles", doc).unwrap();

        let req = SearchRequest {
            query: Some(json!({"match": {"title": "Rust"}})),
            ..Default::default()
        };
        let resp = state.engine.search("articles", req).unwrap();
        assert_eq!(resp.hits.total.value, 1);
        assert!(!resp.hits.hits.is_empty());
        assert!(resp.hits.max_score.unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn module_name_constant() {
        assert_eq!(MODULE_NAME, "cave-search");
        assert_eq!(UPSTREAM, "OpenSearch");
    }
}

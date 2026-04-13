//! cave-vector-search — sovereign vector similarity search engine.
//!
//! Replaces Qdrant with a built-in HNSW index and pluggable distance metrics,
//! with adapter stubs for external backends (Qdrant, Azure AI Search vector).
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────┐
//! │  REST Layer  (Axum, Qdrant-wire-compatible)                │
//! ├───────────────────────────────────────────────────────────┤
//! │  VectorStore trait                                         │
//! │  ├─ BuiltinVectorStore (HNSW + payload filtering)          │
//! │  ├─ QdrantAdapter      (pass-through to Qdrant)            │
//! │  └─ AzureVectorAdapter (Azure AI Search vector)            │
//! ├───────────────────────────────────────────────────────────┤
//! │  HNSW graph index  (multi-layer NSW graph, ef-search)      │
//! ├───────────────────────────────────────────────────────────┤
//! │  Distance metrics  (cosine, euclidean, dot, manhattan)     │
//! └───────────────────────────────────────────────────────────┘
//! ```

pub mod distance;
pub mod engine;
pub mod hnsw;
pub mod models;
pub mod routes;

use async_trait::async_trait;
use axum::Router;
use std::sync::Arc;
use thiserror::Error;

use engine::BuiltinVectorStore;
use models::{
    CollectionConfig, CollectionInfo, Filter, FieldIndexSchema, Point, PointId, RecommendRequest,
    ScrollResponse, SearchRequest, ScoredPoint, SetPayloadRequest, UpdateResult,
};

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum VectorError {
    #[error("collection not found: {0}")]
    CollectionNotFound(String),

    #[error("collection already exists: {0}")]
    CollectionAlreadyExists(String),

    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error("point not found: {0}")]
    PointNotFound(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("filter error: {0}")]
    FilterError(String),

    #[error("backend error: {0}")]
    BackendError(String),

    #[error("serialisation error: {0}")]
    Serialisation(#[from] serde_json::Error),
}

impl axum::response::IntoResponse for VectorError {
    fn into_response(self) -> axum::response::Response {
        let (status, code) = match &self {
            VectorError::CollectionNotFound(_) => {
                (axum::http::StatusCode::NOT_FOUND, "not_found")
            }
            VectorError::CollectionAlreadyExists(_) => {
                (axum::http::StatusCode::CONFLICT, "already_exists")
            }
            VectorError::DimensionMismatch { .. } => {
                (axum::http::StatusCode::BAD_REQUEST, "dimension_mismatch")
            }
            VectorError::PointNotFound(_) => {
                (axum::http::StatusCode::NOT_FOUND, "not_found")
            }
            VectorError::InvalidRequest(_) => {
                (axum::http::StatusCode::BAD_REQUEST, "bad_request")
            }
            _ => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        (
            status,
            axum::Json(serde_json::json!({
                "status": "error",
                "error": {
                    "code": code,
                    "description": self.to_string()
                }
            })),
        )
            .into_response()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VectorStore trait
// ─────────────────────────────────────────────────────────────────────────────

/// The core vector store contract.
///
/// The built-in implementation uses an in-memory HNSW graph.  External
/// adapters (Qdrant, Azure AI Search vector) implement this trait so the
/// platform can swap backends via configuration without touching business
/// logic.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Create a new collection.
    async fn create_collection(
        &self,
        name: &str,
        config: CollectionConfig,
    ) -> Result<(), VectorError>;

    /// Permanently delete a collection and all its points.
    async fn delete_collection(&self, name: &str) -> Result<(), VectorError>;

    /// Return collection metadata and statistics.
    async fn collection_info(&self, name: &str) -> Result<CollectionInfo, VectorError>;

    /// Insert or update a batch of points.
    async fn upsert_points(
        &self,
        collection: &str,
        points: Vec<Point>,
    ) -> Result<UpdateResult, VectorError>;

    /// Retrieve a single point by ID.
    async fn get_point(
        &self,
        collection: &str,
        id: &PointId,
    ) -> Result<Option<Point>, VectorError>;

    /// Delete points by IDs or filter.
    async fn delete_points(
        &self,
        collection: &str,
        ids: Vec<PointId>,
        filter: Option<Filter>,
    ) -> Result<UpdateResult, VectorError>;

    /// Approximate k-nearest-neighbour search.
    async fn search(
        &self,
        collection: &str,
        request: SearchRequest,
    ) -> Result<Vec<ScoredPoint>, VectorError>;

    /// Recommend points similar to positive examples and dissimilar to negatives.
    async fn recommend(
        &self,
        collection: &str,
        request: RecommendRequest,
    ) -> Result<Vec<ScoredPoint>, VectorError>;

    /// Scroll through all (optionally filtered) points in a collection.
    async fn scroll(
        &self,
        collection: &str,
        filter: Option<Filter>,
        limit: usize,
        offset: Option<PointId>,
    ) -> Result<ScrollResponse, VectorError>;

    /// Create a payload field index.
    async fn create_field_index(
        &self,
        collection: &str,
        field: &str,
        schema: FieldIndexSchema,
    ) -> Result<UpdateResult, VectorError>;

    /// List all collection names.
    async fn list_collections(&self) -> Vec<String>;

    /// Update payload on points matching IDs or filter.
    async fn set_payload(
        &self,
        collection: &str,
        req: SetPayloadRequest,
    ) -> Result<UpdateResult, VectorError>;
}

// ─────────────────────────────────────────────────────────────────────────────
// External adapter stubs
// ─────────────────────────────────────────────────────────────────────────────

/// Adapter that proxies requests to an external Qdrant service.
pub struct QdrantAdapter {
    pub endpoint: String,
    pub api_key: Option<String>,
}

impl QdrantAdapter {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into(), api_key: None }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
}

#[async_trait]
impl VectorStore for QdrantAdapter {
    async fn create_collection(&self, name: &str, _config: CollectionConfig) -> Result<(), VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: create_collection({}) not implemented — connect to {}", name, self.endpoint)))
    }
    async fn delete_collection(&self, name: &str) -> Result<(), VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: delete_collection({}) not implemented", name)))
    }
    async fn collection_info(&self, name: &str) -> Result<CollectionInfo, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: collection_info({}) not implemented", name)))
    }
    async fn upsert_points(&self, collection: &str, _points: Vec<Point>) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: upsert_points({}) not implemented", collection)))
    }
    async fn get_point(&self, collection: &str, id: &PointId) -> Result<Option<Point>, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: get_point({}/{}) not implemented", collection, id)))
    }
    async fn delete_points(&self, collection: &str, _ids: Vec<PointId>, _filter: Option<Filter>) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: delete_points({}) not implemented", collection)))
    }
    async fn search(&self, collection: &str, _request: SearchRequest) -> Result<Vec<ScoredPoint>, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: search({}) not implemented", collection)))
    }
    async fn recommend(&self, collection: &str, _request: RecommendRequest) -> Result<Vec<ScoredPoint>, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: recommend({}) not implemented", collection)))
    }
    async fn scroll(&self, collection: &str, _filter: Option<Filter>, _limit: usize, _offset: Option<PointId>) -> Result<ScrollResponse, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: scroll({}) not implemented", collection)))
    }
    async fn create_field_index(&self, collection: &str, _field: &str, _schema: FieldIndexSchema) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: create_field_index({}) not implemented", collection)))
    }
    async fn list_collections(&self) -> Vec<String> { vec![] }
    async fn set_payload(&self, collection: &str, _req: SetPayloadRequest) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("QdrantAdapter: set_payload({}) not implemented", collection)))
    }
}

/// Adapter that proxies requests to Azure AI Search (vector search).
pub struct AzureVectorAdapter {
    pub service_name: String,
    pub api_key: String,
    pub api_version: String,
}

impl AzureVectorAdapter {
    pub fn new(service_name: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            api_key: api_key.into(),
            api_version: "2023-11-01".into(),
        }
    }
}

#[async_trait]
impl VectorStore for AzureVectorAdapter {
    async fn create_collection(&self, name: &str, _config: CollectionConfig) -> Result<(), VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: create_collection({}) not implemented — connect to {}.search.windows.net", name, self.service_name)))
    }
    async fn delete_collection(&self, name: &str) -> Result<(), VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: delete_collection({}) not implemented", name)))
    }
    async fn collection_info(&self, name: &str) -> Result<CollectionInfo, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: collection_info({}) not implemented", name)))
    }
    async fn upsert_points(&self, collection: &str, _points: Vec<Point>) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: upsert_points({}) not implemented", collection)))
    }
    async fn get_point(&self, collection: &str, id: &PointId) -> Result<Option<Point>, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: get_point({}/{}) not implemented", collection, id)))
    }
    async fn delete_points(&self, collection: &str, _ids: Vec<PointId>, _filter: Option<Filter>) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: delete_points({}) not implemented", collection)))
    }
    async fn search(&self, collection: &str, _request: SearchRequest) -> Result<Vec<ScoredPoint>, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: search({}) not implemented", collection)))
    }
    async fn recommend(&self, collection: &str, _request: RecommendRequest) -> Result<Vec<ScoredPoint>, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: recommend({}) not implemented", collection)))
    }
    async fn scroll(&self, collection: &str, _filter: Option<Filter>, _limit: usize, _offset: Option<PointId>) -> Result<ScrollResponse, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: scroll({}) not implemented", collection)))
    }
    async fn create_field_index(&self, collection: &str, _field: &str, _schema: FieldIndexSchema) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: create_field_index({}) not implemented", collection)))
    }
    async fn list_collections(&self) -> Vec<String> { vec![] }
    async fn set_payload(&self, collection: &str, _req: SetPayloadRequest) -> Result<UpdateResult, VectorError> {
        Err(VectorError::BackendError(format!("AzureVectorAdapter: set_payload({}) not implemented", collection)))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// State
// ─────────────────────────────────────────────────────────────────────────────

/// Module state passed through the Axum router.
pub struct VectorState {
    pub store: Arc<BuiltinVectorStore>,
}

impl Default for VectorState {
    fn default() -> Self {
        Self { store: BuiltinVectorStore::new() }
    }
}

impl VectorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Use an external Qdrant cluster.  Currently falls back to the built-in
    /// engine — replace with `Arc<QdrantAdapter>` when fully implemented.
    pub fn with_qdrant(endpoint: impl Into<String>) -> Self {
        tracing::warn!(
            endpoint = endpoint.into().as_str(),
            "QdrantAdapter stub selected — falling back to built-in engine"
        );
        Self::default()
    }
}

/// Module identifier consumed by the portal / health aggregation.
pub const MODULE_NAME: &str = "cave-vector-search";

/// Upstream project this module replaces.
pub const UPSTREAM: &str = "Qdrant";

// ─────────────────────────────────────────────────────────────────────────────
// Public router factory
// ─────────────────────────────────────────────────────────────────────────────

pub fn router(state: Arc<VectorState>) -> Router {
    routes::create_router(state)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vector_state_default() {
        let state = VectorState::default();
        assert!(state.store.list_collections().is_empty());
    }

    #[test]
    fn error_display() {
        let e = VectorError::CollectionNotFound("my-col".into());
        assert!(e.to_string().contains("my-col"));

        let e2 = VectorError::DimensionMismatch { expected: 128, got: 64 };
        assert!(e2.to_string().contains("128"));
    }

    #[test]
    fn module_constants() {
        assert_eq!(MODULE_NAME, "cave-vector-search");
        assert_eq!(UPSTREAM, "Qdrant");
    }

    #[test]
    fn qdrant_adapter_endpoint() {
        let a = QdrantAdapter::new("http://qdrant:6333");
        assert!(a.endpoint.contains("6333"));
        assert!(a.api_key.is_none());
    }

    #[test]
    fn azure_vector_adapter_version() {
        let a = AzureVectorAdapter::new("my-svc", "key-123");
        assert_eq!(a.api_version, "2023-11-01");
    }

    #[test]
    fn full_search_flow() {
        use models::{CollectionConfig, Distance, Point, PointId, SearchRequest};
        use std::collections::HashMap;

        let state = VectorState::default();
        let config = CollectionConfig::new(3, Distance::Cosine);
        state.store.create_collection("embeddings", config).unwrap();

        let points = vec![
            Point::new(PointId::Num(1), vec![1.0, 0.0, 0.0], HashMap::new()),
            Point::new(PointId::Num(2), vec![0.0, 1.0, 0.0], HashMap::new()),
            Point::new(PointId::Num(3), vec![0.0, 0.0, 1.0], HashMap::new()),
        ];
        state.store.upsert_points("embeddings", points).unwrap();

        let req = SearchRequest {
            vector: vec![1.0, 0.0, 0.0],
            limit: 1,
            ..Default::default()
        };
        let results = state.store.search("embeddings", req).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, PointId::Num(1));
    }
}

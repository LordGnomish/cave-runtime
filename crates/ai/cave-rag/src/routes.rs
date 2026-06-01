// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-rag — the offline RAG endpoints cave-runtime merges
//! and the portal page calls.
//!
//! The crate stays library-first: handlers are thin shells over the same
//! pure functions the [`cli`](crate::cli) uses, so the HTTP surface adds no
//! new business logic. State is intentionally empty — every endpoint is a
//! stateless transform of its request body — but a [`RagState`] is threaded
//! through for parity with the other cave-runtime modules' mount signature.

use std::sync::Arc;

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::graphrag::extract_graph;
use crate::splitter::RecursiveCharacterTextSplitter;

/// Shared state for the RAG router (currently stateless).
#[derive(Debug, Clone, Default)]
pub struct RagState {}

/// Build the axum router exposing the cave-rag HTTP API.
pub fn router(state: Arc<RagState>) -> Router {
    Router::new()
        .route("/api/rag/health", get(health))
        .route("/api/rag/graph/extract", post(graph_extract))
        .route("/api/rag/graph/search", post(graph_search))
        .route("/api/rag/split", post(split))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "module": "cave-rag",
        "upstreams": ["langchain", "llama_index", "graphrag", "haystack"],
        "endpoints": [
            "/api/rag/graph/extract",
            "/api/rag/graph/search",
            "/api/rag/split",
        ],
    }))
}

#[derive(Debug, Deserialize)]
struct ExtractReq {
    text: String,
}

async fn graph_extract(
    State(_): State<Arc<RagState>>,
    Json(req): Json<ExtractReq>,
) -> Json<Value> {
    Json(graph_extract_value(&req.text))
}

#[derive(Debug, Deserialize)]
struct SearchReq {
    text: String,
    query: String,
    #[serde(default = "one")]
    hops: usize,
}

fn one() -> usize {
    1
}

async fn graph_search(State(_): State<Arc<RagState>>, Json(req): Json<SearchReq>) -> Json<Value> {
    let g = extract_graph(&req.text);
    let context = g.local_search(&req.query, req.hops);
    Json(json!({ "query": req.query, "hops": req.hops, "context": context }))
}

#[derive(Debug, Deserialize)]
struct SplitReq {
    text: String,
    #[serde(default = "default_size")]
    size: usize,
    #[serde(default)]
    overlap: usize,
}

fn default_size() -> usize {
    400
}

async fn split(State(_): State<Arc<RagState>>, Json(req): Json<SplitReq>) -> Json<Value> {
    let chunks = RecursiveCharacterTextSplitter::default()
        .with_chunk_size(req.size)
        .with_chunk_overlap(req.overlap)
        .split_text(&req.text);
    Json(json!({ "chunk_count": chunks.len(), "chunks": chunks }))
}

/// Pure graph-extraction transform shared by the HTTP and CLI surfaces.
fn graph_extract_value(text: &str) -> Value {
    let g = extract_graph(text);
    let communities = g.communities();
    let mut entities: Vec<String> = communities.iter().flatten().cloned().collect();
    entities.sort();
    entities.dedup();
    let relationships: Vec<Value> = g
        .relationships()
        .iter()
        .map(|r| json!({"source": r.source, "relation": r.relation, "target": r.target}))
        .collect();
    json!({
        "entities": entities,
        "entity_count": g.entity_count(),
        "relationships": relationships,
        "relationship_count": g.relationship_count(),
        "communities": communities,
    })
}

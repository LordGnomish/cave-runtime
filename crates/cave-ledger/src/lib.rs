//! CAVE Sovereign Ledger — immutable audit log with Merkle tree hash chain.
//!
//! Implements Principle 14 (Sovereign Auditability), ADR-093.
//!
//! Every platform operation is recorded as a LedgerEntry with:
//! - SHA-256 hash of the entry content
//! - Previous entry's hash (forming a chain)
//! - Merkle root of the current batch
//! - Timestamp, actor, action, and metadata
//!
//! Storage backends:
//! - WORM-capable object storage (MinIO/ADLS) for production
//! - Local file for development
//! - In-memory for testing
//!
//! Integration points:
//! - CI pipeline signs entries with Sigstore (cosign keyless)
//! - Emergency CLI creates signed commits referencing ledger hashes
//! - Compliance export reads from ledger for SOC2/ISO27001 evidence

pub mod chain;
pub mod entry;
pub mod store;

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;

pub use chain::MerkleChain;
pub use entry::{LedgerEntry, LedgerEntryKind};

/// Shared state for the Ledger module.
pub struct LedgerState {
    pub chain: RwLock<MerkleChain>,
}

impl Default for LedgerState {
    fn default() -> Self {
        Self {
            chain: RwLock::new(MerkleChain::new()),
        }
    }
}

/// Create the Axum router for ledger endpoints.
pub fn router(state: Arc<LedgerState>) -> Router {
    Router::new()
        .route("/api/ledger/health", get(health))
        .route("/api/ledger/entries", get(list_entries))
        .route("/api/ledger/entries", post(append_entry))
        .route("/api/ledger/verify", get(verify_chain))
        .route("/api/ledger/stats", get(stats))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-ledger",
        "status": "healthy",
    }))
}

async fn list_entries(
    State(state): State<Arc<LedgerState>>,
) -> Json<serde_json::Value> {
    let chain = state.chain.read().await;
    let entries: Vec<_> = chain
        .entries()
        .iter()
        .map(|e| {
            serde_json::json!({
                "sequence": e.sequence,
                "hash": e.hash,
                "previous_hash": e.previous_hash,
                "kind": e.kind,
                "actor": e.actor,
                "action": e.action,
                "timestamp": e.timestamp,
            })
        })
        .collect();

    Json(serde_json::json!({
        "total": entries.len(),
        "entries": entries,
    }))
}

async fn append_entry(
    State(state): State<Arc<LedgerState>>,
    Json(req): Json<AppendRequest>,
) -> Json<serde_json::Value> {
    let mut chain = state.chain.write().await;
    let entry = chain.append(req.kind, &req.actor, &req.action, req.metadata);

    Json(serde_json::json!({
        "sequence": entry.sequence,
        "hash": entry.hash,
        "previous_hash": entry.previous_hash,
    }))
}

async fn verify_chain(
    State(state): State<Arc<LedgerState>>,
) -> Json<serde_json::Value> {
    let chain = state.chain.read().await;
    let result = chain.verify();

    Json(serde_json::json!({
        "valid": result.is_valid,
        "entries_checked": result.entries_checked,
        "error": result.error,
    }))
}

async fn stats(
    State(state): State<Arc<LedgerState>>,
) -> Json<serde_json::Value> {
    let chain = state.chain.read().await;
    let entries = chain.entries();

    let mut kind_counts = std::collections::HashMap::new();
    for entry in entries {
        *kind_counts
            .entry(format!("{:?}", entry.kind))
            .or_insert(0u64) += 1;
    }

    Json(serde_json::json!({
        "total_entries": entries.len(),
        "merkle_root": chain.merkle_root(),
        "kind_counts": kind_counts,
    }))
}

#[derive(serde::Deserialize)]
struct AppendRequest {
    kind: LedgerEntryKind,
    actor: String,
    action: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

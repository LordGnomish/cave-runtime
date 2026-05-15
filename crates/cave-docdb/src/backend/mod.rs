// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: FerretDB/FerretDB@27653503a28b0d2f2a09d7105887edd3a8af1060 internal/backends/backend.go
//! Pluggable backend abstraction for cave-docdb.
//!
//! Mirrors the FerretDB `internal/backends` interface split: a `Backend`
//! trait that hosts all storage primitives (insert/find/update/delete +
//! collection / database listing), with concrete implementations for an
//! in-memory store (the canonical reference) and a PostgreSQL adapter
//! (`PostgresBackend`) that maps MongoDB collections onto JSONB tables.
//!
//! The clean-room oplog / change-stream / shard modules consume this
//! trait by composition so they never depend on a concrete storage
//! engine — and so swapping memory ↔ postgres requires no rewrites.

use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod memory;
pub mod postgres;

pub use memory::MemoryBackend;
pub use postgres::{PostgresBackend, PostgresConfig};

/// Document alias — a `BTreeMap<String, Value>` matching `crate::bson::Document`.
pub type BsonDoc = BTreeMap<String, serde_json::Value>;

/// Write-path result; mirrors the MongoDB `nModified`/`n` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteResult {
    pub affected: u64,
}

/// Errors any backend may surface.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("collection {db}.{coll} not found")]
    CollectionNotFound { db: String, coll: String },
    #[error("database {0} not found")]
    DatabaseNotFound(String),
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("invalid document: {0}")]
    InvalidDocument(String),
    #[error("io: {0}")]
    Io(String),
}

/// Pluggable storage backend. Implementations must be `Send + Sync` and
/// safe to share via `Arc`.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Return the name tag of the backend (e.g. `"memory"`, `"postgres"`).
    fn name(&self) -> &'static str;

    /// Insert one or more documents into `db.coll`. Auto-creates the
    /// collection if absent.
    async fn insert(
        &self,
        db: &str,
        coll: &str,
        docs: Vec<BsonDoc>,
    ) -> Result<WriteResult, BackendError>;

    /// Find documents matching `filter` (or every doc if `None`).
    async fn find(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<Vec<BsonDoc>, BackendError>;

    /// Update all docs matching `filter` with the update spec.
    async fn update(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
        update: BsonDoc,
    ) -> Result<WriteResult, BackendError>;

    /// Delete all docs matching `filter`.
    async fn delete(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<WriteResult, BackendError>;

    /// Count docs matching `filter`.
    async fn count(
        &self,
        db: &str,
        coll: &str,
        filter: Option<BsonDoc>,
    ) -> Result<u64, BackendError>;

    /// List collection names in `db`.
    async fn list_collections(&self, db: &str) -> Result<Vec<String>, BackendError>;

    /// List database names known to the backend.
    async fn list_databases(&self) -> Result<Vec<String>, BackendError>;

    /// Drop a collection — silent no-op if absent.
    async fn drop_collection(&self, db: &str, coll: &str) -> Result<(), BackendError>;

    /// Drop a database — silent no-op if absent.
    async fn drop_database(&self, db: &str) -> Result<(), BackendError>;
}

/// Shared backend handle.
pub type SharedBackend = Arc<dyn Backend>;

/// Process-wide registry of named backends — used by the HTTP route layer
/// when the operator wires e.g. `tenant_a → PostgresBackend`,
/// `tenant_b → MemoryBackend` via config.
#[derive(Default, Clone)]
pub struct BackendRegistry {
    inner: Arc<RwLock<Vec<(String, SharedBackend)>>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, name: impl Into<String>, b: SharedBackend) {
        let mut g = self.inner.write().await;
        let name = name.into();
        // last-write-wins: replace if name exists.
        if let Some(slot) = g.iter_mut().find(|(n, _)| n == &name) {
            slot.1 = b;
        } else {
            g.push((name, b));
        }
    }

    pub async fn get(&self, name: &str) -> Option<SharedBackend> {
        self.inner
            .read()
            .await
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| Arc::clone(b))
    }

    pub async fn names(&self) -> Vec<String> {
        self.inner.read().await.iter().map(|(n, _)| n.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registry_roundtrip() {
        let r = BackendRegistry::new();
        let b: SharedBackend = Arc::new(MemoryBackend::new());
        r.register("primary", b).await;
        assert_eq!(r.names().await, vec!["primary".to_string()]);
        assert!(r.get("primary").await.is_some());
        assert!(r.get("missing").await.is_none());
    }

    #[tokio::test]
    async fn registry_replace_on_collision() {
        let r = BackendRegistry::new();
        r.register("p", Arc::new(MemoryBackend::new())).await;
        r.register("p", Arc::new(MemoryBackend::new())).await;
        assert_eq!(r.names().await.len(), 1);
    }
}

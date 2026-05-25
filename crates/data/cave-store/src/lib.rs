// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Store — Unified storage engine.
//!
//! Compatible with: etcd (KV store) + MinIO (S3 object storage).
//!
//! Features:
//! - Full etcd v3 HTTP/JSON API (KV, Watch, Lease, Auth, Cluster)
//! - Full S3/MinIO-compatible REST API
//! - File-based storage with Write-Ahead Log (WAL) for crash recovery
//! - MVCC (Multi-Version Concurrency Control) for etcd semantics
//! - SSE-S3 and SSE-C encryption for objects
//! - Bucket versioning, lifecycle rules, notifications, policies

pub mod engine;
pub mod error;
pub mod etcd;
pub mod s3;
pub mod wal;

use axum::Router;
use etcd::auth::AuthManager;
use etcd::cluster::ClusterManager;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

pub use error::{StoreError, StoreResult};

/// Shared state for the entire cave-store module.
pub struct StoreState {
    /// etcd-compatible MVCC key-value engine
    pub engine: Arc<engine::MvccEngine>,
    /// S3/MinIO object store
    pub s3: Arc<s3::ObjectStore>,
    /// Auth manager (etcd auth API)
    pub auth: Arc<AuthManager>,
    /// Cluster manager (etcd cluster API)
    pub cluster: Arc<ClusterManager>,
    /// Secret key for presigned URL signing
    pub s3_secret_key: String,
}

impl StoreState {
    /// Open or create a store rooted at `data_dir`, replaying the WAL.
    pub async fn open(data_dir: PathBuf) -> StoreResult<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)?;

        // Read WAL entries for replay
        let existing_entries = wal::read_wal(&data_dir)?;
        info!(entries = existing_entries.len(), "Replaying WAL");

        // Open WAL writer (append mode)
        let wal_writer = wal::WalWriter::open(&data_dir)?;
        let wal_arc = Arc::new(wal_writer);

        // Build MVCC engine and replay etcd entries
        let engine = Arc::new(engine::MvccEngine::new(
            wal::WalWriter::open(&data_dir)?, // separate writer instance
        ));
        engine.replay_wal(existing_entries.clone()).await;

        // Build S3 store and replay S3 WAL entries
        let s3_data = data_dir.join("objects");
        std::fs::create_dir_all(&s3_data)?;
        let s3 = Arc::new(s3::ObjectStore::new(s3_data, wal_arc));
        s3.replay_wal(&existing_entries).await;

        let state = Arc::new(Self {
            engine: engine.clone(),
            s3: s3.clone(),
            auth: Arc::new(AuthManager::default()),
            cluster: Arc::new(ClusterManager::default()),
            s3_secret_key: std::env::var("CAVE_STORE_SECRET")
                .unwrap_or_else(|_| "cave-store-dev-secret-key".to_string()),
        });

        // Spawn background tasks
        let engine_clone = engine.clone();
        tokio::spawn(engine::MvccEngine::run_lease_reaper(engine_clone));

        let s3_clone = s3.clone();
        tokio::spawn(s3::ObjectStore::run_lifecycle_enforcer(s3_clone));

        Ok(state)
    }

    /// Create an in-memory store for testing (no WAL persistence).
    pub fn in_memory() -> Arc<Self> {
        let data_dir =
            std::env::temp_dir().join(format!("cave-store-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let wal = wal::WalWriter::open(&data_dir).unwrap();
        let wal_arc = Arc::new(wal::WalWriter::open(&data_dir).unwrap());
        let s3_data = data_dir.join("objects");
        std::fs::create_dir_all(&s3_data).unwrap();

        Arc::new(Self {
            engine: Arc::new(engine::MvccEngine::new(wal)),
            s3: Arc::new(s3::ObjectStore::new(s3_data, wal_arc)),
            auth: Arc::new(AuthManager::default()),
            cluster: Arc::new(ClusterManager::default()),
            s3_secret_key: "test-secret".to_string(),
        })
    }
}

/// Build the unified router for both etcd and S3 APIs.
pub fn router(state: Arc<StoreState>) -> Router {
    Router::new()
        // etcd v3 API at /v3/*
        .merge(etcd::etcd_router(state.clone()))
        // S3/MinIO API at /s3/*
        .nest("/s3", s3::s3_router(state))
}

pub const MODULE_NAME: &str = "store";

#[cfg(test)]
mod tests;

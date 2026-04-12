//! CAVE Backup — Kubernetes backup/restore engine.
//!
//! Replaces: Velero
//! Features: full/namespace/label-selector backup, S3/Azure/GCS/local targets,
//! cron schedules with retention, CSI snapshots, restic/kopia file backup,
//! pre/post hooks, cross-cluster restore, AES-256 encryption.

pub mod engine;
pub mod hooks;
pub mod routes;
pub mod models;
pub mod engine;
pub mod schedule;
pub mod storage;
pub mod types;
pub mod volume;
pub mod models;
pub mod engine;

use axum::Router;
use std::sync::Arc;

pub struct BackupState {
    pub store: Arc<storage::BackupStore>,
}

impl BackupState {
    pub fn new() -> Self {
        Self {
            store: Arc::new(storage::BackupStore::new()),
        }
    }
}

impl Default for BackupState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<BackupState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "backup";

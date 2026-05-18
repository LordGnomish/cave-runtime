// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Backup — Kubernetes backup/restore engine.
//!
//! Compatible with: Velero
//! Upstream tracking: see cave-upstream for monitored features.
//! Provides backup scheduling, storage location management, filesystem
//! backup via restic/kopia, and garbage collection of expired backups.

pub mod engine;
pub mod filesystem;
pub mod gc;
pub mod hooks;
pub mod models;
pub mod routes;
pub mod schedule;
pub mod storage;

use axum::Router;
use models::{
    BackupStorageLocation, BslAccessMode, BslPhase, StorageProvider,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// All in-memory collections for the backup module.
pub struct BackupStore {
    pub backups: HashMap<Uuid, models::Backup>,
    pub restores: HashMap<Uuid, models::Restore>,
    pub schedules: HashMap<Uuid, models::Schedule>,
    pub storage_locations: HashMap<Uuid, models::BackupStorageLocation>,
    pub volume_snapshot_locations: HashMap<Uuid, models::VolumeSnapshotLocation>,
    pub fs_backup_jobs: HashMap<Uuid, models::FsBackupJob>,
}

impl Default for BackupStore {
    fn default() -> Self {
        let mut storage_locations = HashMap::new();
        let default_id = Uuid::new_v4();
        let default_bsl = BackupStorageLocation {
            id: default_id,
            name: "default".into(),
            provider: StorageProvider::S3,
            bucket: "cave-backups".into(),
            prefix: None,
            region: Some("us-east-1".into()),
            endpoint: None,
            access_mode: BslAccessMode::ReadWrite,
            credential_secret: None,
            ca_bundle: None,
            insecure_skip_tls_verify: false,
            is_default: true,
            phase: BslPhase::Available,
            last_validated_at: None,
            created_at: chrono::Utc::now(),
        };
        storage_locations.insert(default_id, default_bsl);

        Self {
            backups: HashMap::new(),
            restores: HashMap::new(),
            schedules: HashMap::new(),
            storage_locations,
            volume_snapshot_locations: HashMap::new(),
            fs_backup_jobs: HashMap::new(),
        }
    }
}

/// Shared state for the backup module, protected by a tokio RwLock.
pub struct BackupState {
    pub store: Arc<RwLock<BackupStore>>,
}

impl Default for BackupState {
    fn default() -> Self {
        Self {
            store: Arc::new(RwLock::new(BackupStore::default())),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<BackupState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "backup";

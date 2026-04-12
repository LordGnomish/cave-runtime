//! cave-store — MinIO replacement for object storage management.
//!
//! Replaces: MinIO, AWS S3 (dev/platform use)
//! Features: bucket CRUD, put/get/delete objects, multipart upload,
//!           versioning, lifecycle rules, access policies, replication rules.

pub mod models;
pub mod routes;
pub mod storage;

use axum::Router;
use std::sync::{Arc, Mutex};

/// Shared state for the object store module.
pub struct StoreState {
    pub inner: Mutex<storage::ObjectStore>,
}

impl StoreState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(storage::ObjectStore::new()),
        }
    }
}

impl Default for StoreState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<StoreState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "store";

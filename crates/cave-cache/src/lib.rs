//! cave-cache — Redis/Valkey replacement for distributed caching.
//!
//! Replaces: Redis, Valkey
//! Features: get/set/delete/expire, glob pattern matching, atomic incr/decr,
//!           pipeline operations, pub/sub channels, in-memory TTL eviction.

pub mod cache;
pub mod models;
pub mod routes;

use axum::Router;
use std::sync::{Arc, Mutex};

/// Shared state for the cache module.
pub struct CacheState {
    pub store: Mutex<cache::CacheStore>,
}

impl CacheState {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(cache::CacheStore::new()),
        }
    }
}

impl Default for CacheState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<CacheState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "cache";

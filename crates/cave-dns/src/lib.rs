//! CAVE DNS — DNS record management.
//!
//! Replaces: external-dns
//! Multi-provider DNS sync, drift detection, record validation, health probes.

pub mod manager;
pub mod models;
pub mod routes;

use axum::Router;
use models::{DnsRecord, DnsZone};
use std::sync::{Arc, Mutex};

pub struct DnsState {
    pub zones: Arc<Mutex<Vec<DnsZone>>>,
    pub records: Arc<Mutex<Vec<DnsRecord>>>,
}

impl Default for DnsState {
    fn default() -> Self {
        Self {
            zones: Arc::new(Mutex::new(Vec::new())),
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

pub fn router(state: Arc<DnsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "dns";

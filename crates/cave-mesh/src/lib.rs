//! CAVE Mesh — Service mesh replacing Istio + Linkerd.
//!
//! Replaces: Istio, Linkerd
//! Features: mTLS, traffic splitting, canary routing, circuit breaking,
//!           fault injection, traffic mirroring, golden-signal observability.

pub mod models;
pub mod mtls;
pub mod observability;
pub mod proxy;
pub mod routes;
pub mod traffic;

use axum::Router;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

/// Shared in-memory state for the mesh module.
/// Each collection is independently locked to reduce contention.
pub struct MeshState {
    pub services: Mutex<HashMap<Uuid, models::Service>>,
    pub instances: Mutex<HashMap<Uuid, models::ServiceInstance>>,
    pub virtual_services: Mutex<HashMap<Uuid, models::VirtualService>>,
    pub traffic_policies: Mutex<HashMap<Uuid, models::TrafficPolicy>>,
    pub destination_rules: Mutex<HashMap<Uuid, models::DestinationRule>>,
    pub service_entries: Mutex<HashMap<Uuid, models::ServiceEntry>>,
    pub sidecars: Mutex<HashMap<Uuid, models::SidecarConfig>>,
    pub circuit_breakers: Mutex<HashMap<Uuid, proxy::CircuitBreakerState>>,
    pub metrics: Mutex<HashMap<Uuid, observability::ServiceMetrics>>,
    pub certs: Mutex<HashMap<Uuid, mtls::CertRecord>>,
}

impl Default for MeshState {
    fn default() -> Self {
        Self {
            services: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
            virtual_services: Mutex::new(HashMap::new()),
            traffic_policies: Mutex::new(HashMap::new()),
            destination_rules: Mutex::new(HashMap::new()),
            service_entries: Mutex::new(HashMap::new()),
            sidecars: Mutex::new(HashMap::new()),
            circuit_breakers: Mutex::new(HashMap::new()),
            metrics: Mutex::new(HashMap::new()),
            certs: Mutex::new(HashMap::new()),
        }
    }
}

/// Create the axum router for the mesh module.
pub fn router(state: Arc<MeshState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "mesh";

//! TLS certificate lifecycle — compatible with cert-manager
//!
//! Upstream tracking: cert-manager
//! Features: ACME/Lets Encrypt, cert issuance, auto-renewal, expiry alerting, K8s CRDs

pub mod routes;
pub mod models;
pub mod engine;
pub mod crds;
pub mod acme_client;
pub mod solvers;
pub mod renewal;
pub mod pqc;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "certs";

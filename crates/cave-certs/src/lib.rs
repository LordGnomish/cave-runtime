//! TLS certificate lifecycle — replaces cert-manager
//!
//! Upstream tracking: cert-manager
//! Features: ACME/Lets Encrypt, cert issuance, auto-renewal, expiry alerting, K8s CRDs

pub mod routes;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "certs";

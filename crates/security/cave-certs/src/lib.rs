// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TLS certificate lifecycle — compatible with cert-manager
//!
//! Upstream tracking: cert-manager
//! Features: ACME/Lets Encrypt, cert issuance, auto-renewal, expiry alerting, K8s CRDs

pub mod acme_client;
pub mod cert_request;
pub mod crds;
pub mod engine;
pub mod models;
pub mod pqc;
pub mod renewal;
pub mod routes;
pub mod solvers;
pub mod store;

use axum::Router;

pub fn router() -> Router {
    routes::create_router()
}

pub const MODULE_NAME: &str = "certs";

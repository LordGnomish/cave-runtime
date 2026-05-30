// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! # Cave Kamaji
//!
//! This crate provides the core state management and routing for the Cave Kamaji service.
//! It handles tenant lifecycle operations via Axum routes and maintains tenant state in memory.

pub mod certs;
pub mod cluster_api;
pub mod datastore;
pub mod konnectivity;
pub mod kubeadm;
pub mod lifecycle;
pub mod models;
pub mod pod_mgmt;
pub mod routes;
pub mod status;
pub mod webhook;

use axum::{
    Router,
    routing::{get, post},
};
use dashmap::DashMap;
use models::TenantControlPlane;
use std::sync::Arc;
use uuid::Uuid;

/// Represents the global state of the Kamaji service.
///
/// This struct holds the in-memory map of tenants, keyed by their UUID.
/// It is wrapped in an `Arc` for shared ownership across async tasks.
pub struct KamajiState {
    /// A concurrent hash map storing tenant control planes.
    pub tenants: DashMap<Uuid, TenantControlPlane>,
}

/// Implements the `Default` trait for `KamajiState`.
///
/// Creates a new instance with an empty `DashMap` for tenants.
impl Default for KamajiState {
    fn default() -> Self {
        Self {
            tenants: DashMap::new(),
        }
    }
}

/// Creates the Axum `Router` for the Kamaji API.
///
/// Configures routes for tenant CRUD operations and kubeconfig retrieval.
/// Attaches the provided `KamajiState` to the router for request handling.
pub fn router(state: Arc<KamajiState>) -> Router {
    Router::new()
        .route(
            "/api/kamaji/tenants",
            post(routes::create_tenant).get(routes::list_tenants),
        )
        .route(
            "/api/kamaji/tenants/{id}",
            get(routes::get_tenant).delete(routes::delete_tenant),
        )
        .route(
            "/api/kamaji/tenants/{id}/kubeconfig",
            post(routes::get_kubeconfig),
        )
        .with_state(state)
}

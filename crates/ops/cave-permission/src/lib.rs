// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-permission — Permission framework compatible with Backstage permission-backend.
//!
//! Implements:
//! - `models`    — Permission, ResourcePermission, AuthorizeResult, PolicyDecision (from @backstage/permission-common)
//! - `policy`    — PermissionPolicy trait + AllowAllPermissionPolicy (from @backstage/permission-node)
//! - `routes`    — POST /api/permission/authorize, GET /api/permission/health
//! - `catalog`   — Catalog permission constants (from @backstage/catalog-backend)
//! - `matchers`  — Casbin built-in matcher operators (keyMatch/keyMatch2/keyMatch3/regexMatch/ipMatch)

pub mod catalog;
pub mod matchers;
pub mod models;
pub mod policy;
pub mod routes;

use axum::Router;
use std::sync::Arc;

use policy::{AllowAllPermissionPolicy, PermissionPolicy};

/// Shared application state — holds the active permission policy.
pub struct PermissionState {
    pub policy: Arc<dyn PermissionPolicy>,
}

impl Default for PermissionState {
    fn default() -> Self {
        Self {
            policy: Arc::new(AllowAllPermissionPolicy),
        }
    }
}

/// Build the axum Router for the permission service.
pub fn router(state: Arc<PermissionState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "permission";

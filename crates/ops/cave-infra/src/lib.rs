// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Infra — LLM + MCP-driven Infrastructure-as-Code engine.
//!
//! Compatible with: Terraform, Pulumi, custom IaC tooling
//!
//! Features:
//! - Natural language to infrastructure (describe → provision)
//! - MCP tool protocol for infrastructure actions
//! - Provider abstraction (bare metal, cloud-like)
//! - Resource state tracking (desired vs actual)
//! - Drift detection and reconciliation
//! - Change plans (preview before apply)
//! - Resource dependency graph (topological ordering)
//! - Rollback support
//! - Infrastructure modules/templates

pub mod drift;
pub mod error;
pub mod graph;
pub mod mcp;
pub mod nlp;
pub mod plan;
pub mod provider;
pub mod resource;
pub mod rollback;
pub mod routes;
pub mod templates;

use axum::Router;
use std::sync::Arc;

pub use error::{InfraError, InfraResult};
pub use resource::ResourceStore;

pub const MODULE_NAME: &str = "infra";

/// Shared state for the infra module.
pub struct InfraState {
    pub store: Arc<ResourceStore>,
}

impl Default for InfraState {
    fn default() -> Self {
        Self {
            store: Arc::new(ResourceStore::new()),
        }
    }
}

/// Build Axum router for the IaC + MCP API.
pub fn router(state: Arc<InfraState>) -> Router {
    routes::create_router(state)
}

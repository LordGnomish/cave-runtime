// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CAVE CRM — Sovereign customer relationship management.
//!
//! Upstream: Twenty (twentyhq/twenty) v2.6.0. Standalone, independent
//! from cave-erp's CRM submodule (which is removed in this commit per
//! ADR-145 — see PARITY_REPORT.md "Deprecation absorption").
//!
//! Function-based crate naming per ADR-RUNTIME-UPSTREAM-MIRROR-001.
//! Tenant isolation per ADR-MULTI-TENANT-001 (Kamaji vCluster boundary
//! for hard isolation; in-process `Workspace` for soft isolation inside
//! a single vCluster).
//!
//! ## Quick start
//! ```no_run
//! # async fn run() {
//! use cave_crm::{new_state, models::*};
//! let store = new_state();
//! let ws = store.bootstrap_workspace("Acme").await;
//! let _person = Person::new(ws.id, "Ada", "Lovelace");
//! # }
//! ```

pub mod graphql_schema;
pub mod indexes;
pub mod models;
pub mod routes;
pub mod store;
pub mod webhook;
pub mod workflow;

pub use store::{ConvertedLead, CrmStore};
pub use webhook::{WebhookBus, WebhookDelivery, WebhookOperation, WebhookSubscription};
pub use workflow::{Workflow, WorkflowRun, WorkflowRunStatus, WorkflowStep, WorkflowStepType, WorkflowStore};

pub type State = CrmStore;

use std::sync::Arc;

pub fn router(state: Arc<CrmStore>) -> axum::Router {
    routes::create_router(state)
}

pub fn new_state() -> Arc<CrmStore> {
    Arc::new(CrmStore::default())
}

pub const MODULE_NAME: &str = "crm";
pub const UPSTREAM: &str = "twentyhq/twenty";
pub const UPSTREAM_VERSION: &str = "v2.6.0";

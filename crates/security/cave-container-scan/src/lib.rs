// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Container, IaC, filesystem, secret, and malware scanner — compatible with Trivy.
//!
//! Compatible with: Aqua Trivy (Apache-2.0). Sovereign-safe.

pub mod engine;
pub mod models;
pub mod policy;
pub mod routes;
pub mod scanners;
pub mod vex;

pub use engine::{ScanError, ScanOrchestrator, Scanner};
pub use routes::ContainerScanStore;

use axum::Router;
use std::sync::Arc;

/// Create the axum router for this module.
pub fn router(state: Arc<ContainerScanStore>) -> Router {
    routes::create_router(state)
}

/// Convenience: build a fresh `ContainerScanStore` wrapped in an `Arc`.
pub fn new_state() -> Arc<ContainerScanStore> {
    Arc::new(ContainerScanStore::default())
}

pub const MODULE_NAME: &str = "container-scan";
pub type State = ContainerScanStore;

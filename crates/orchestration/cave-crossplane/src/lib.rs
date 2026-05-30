// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Crossplane — Crossplane v2 control plane.
//!
//! Upstream: crossplane/crossplane v2.3.1
//!
//! Features: XRDs, Compositions (v2 pipeline + legacy resources mode),
//! Composite Resources, Claims, Composed resources, Providers + ProviderConfigs
//! + DeploymentRuntime, Functions (gRPC codec + built-in patch-and-transform /
//! KCL / go-templating / auto-ready), XPKG packages, condition propagation,
//! cave-flavored built-in providers (provider-kubernetes, provider-helm).

pub mod claim;
pub mod cli;
pub mod composition;
pub mod conditions;
pub mod engine;
pub mod environment;
pub mod error;
pub mod function;
pub mod models;
pub mod observability;
pub mod provider;
pub mod providers_builtin;
pub mod reconciler;
pub mod routes;
pub mod xpkg;
pub mod xr;
pub mod xrd;

use axum::Router;
use std::sync::Arc;

pub use claim::ClaimStore;
pub use composition::CompositionStore;
pub use engine::CompositionEngine;
pub use error::{CrossplaneError, CrossplaneResult};
pub use function::FunctionStore;
pub use provider::ProviderStore;
pub use reconciler::ReconcileQueue;
pub use xrd::XrdStore;

pub const MODULE_NAME: &str = "crossplane";

pub struct CrossplaneState {
    pub xrd_store: Arc<XrdStore>,
    pub composition_store: Arc<CompositionStore>,
    pub claim_store: Arc<ClaimStore>,
    pub provider_store: Arc<ProviderStore>,
    pub reconcile_queue: Arc<ReconcileQueue>,
    pub engine: Arc<CompositionEngine>,
    pub function_store: Arc<FunctionStore>,
}

impl Default for CrossplaneState {
    fn default() -> Self {
        Self {
            xrd_store: Arc::new(XrdStore::new()),
            composition_store: Arc::new(CompositionStore::new()),
            claim_store: Arc::new(ClaimStore::new()),
            provider_store: Arc::new(ProviderStore::new()),
            reconcile_queue: Arc::new(ReconcileQueue::new()),
            engine: Arc::new(CompositionEngine::new()),
            function_store: Arc::new(FunctionStore::new()),
        }
    }
}

pub fn router(state: Arc<CrossplaneState>) -> Router {
    routes::create_router(state)
}

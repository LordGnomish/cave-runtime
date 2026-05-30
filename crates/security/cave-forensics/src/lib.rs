// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runtime forensics — Tetragon v1.7.0 deep-port.
//!
//! Upstream: cilium/tetragon `v1.7.0` (`1de2ed8ebea18e56257dc59597aa13bf8f0e471e`).
//!
//! cave-forensics implements the TracingPolicy CRD + kernel event taxonomy +
//! policy filter (matchPIDs/Namespaces/Capabilities/Binaries/Args/Actions)
//! + Enforcer state machine + gRPC/NDJSON exporters and layers a case +
//! chain-of-custody store on top so events survive WORM rotation.

use std::sync::Arc;

pub mod case;
pub mod cli;
pub mod enforcer;
pub mod engine;
pub mod error;
pub mod events;
pub mod evidence;
pub mod export;
pub mod filter;
pub mod models;
pub mod observability;
pub mod parity_self_audit;
pub mod process;
pub mod routes;
pub mod selectors;
pub mod sift;
pub mod store;
pub mod tracing_policy;

use axum::Router;

/// Module HTTP state — exposes the case + policy stores via Arc.
#[derive(Default)]
pub struct State {
    pub cases: Arc<case::CaseStore>,
    pub policies: Arc<store::PolicyStore>,
    pub enforcer: Arc<tokio::sync::RwLock<enforcer::Enforcer>>,
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "forensics";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_name_constant() {
        assert_eq!(MODULE_NAME, "forensics");
    }

    #[test]
    fn test_state_default_creates_stores() {
        let s = State::default();
        assert_eq!(s.cases.count(), 0);
        assert_eq!(s.policies.count(), 0);
    }

    #[test]
    fn test_router_constructs() {
        let s = Arc::new(State::default());
        let _r = router(s);
    }
}

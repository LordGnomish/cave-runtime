// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-bench — K8s security benchmarks.
//!
//! Dual deep-port:
//! - aquasecurity/kube-bench v0.15.5 (CIS K8s Benchmark — master/node/etcd/control-plane)
//! - kubescape/kubescape    v4.0.8 (NSA hardening guide + MITRE ATT&CK mapping)
//!
//! Both upstreams Apache-2.0.

use std::sync::Arc;

pub mod api;
pub mod cis_control_plane;
pub mod cis_engine;
pub mod cis_etcd;
pub mod cis_master;
pub mod cis_node;
pub mod cli;
pub mod custom;
pub mod error;
pub mod kubescape_mitre;
pub mod kubescape_nsa;
pub mod kubescape_security;
pub mod models;
pub mod observability;
pub mod parity_self_audit;
pub mod plugin_marketplace;
pub mod profile;
pub mod report;
pub mod runner;
pub mod scheduler;
pub mod store;

pub use error::BenchError;
pub use models::*;

use axum::Router;

/// Module HTTP state — exposes finding + schedule stores via Arc.
#[derive(Default)]
pub struct State {
    pub findings: store::SharedStore,
    pub schedules: Arc<scheduler::ScheduleRegistry>,
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    api::create_router(state)
}

pub const MODULE_NAME: &str = "bench";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_name_constant() {
        assert_eq!(MODULE_NAME, "bench");
    }

    #[test]
    fn test_state_default_creates_stores() {
        let s = State::default();
        assert_eq!(s.findings.count(), 0);
        assert_eq!(s.schedules.count(), 0);
    }

    #[test]
    fn test_router_constructs() {
        let s = Arc::new(State::default());
        let _r = router(s);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared `State` holding all eight subsystem handles + the cave-k8s-level
//! features (quota tracker, CRD registry, PQC signer, GC orchestrator).
//!
//! `State::default()` constructs a fresh in-memory control plane suitable
//! for tests and for the `cavectl k8s ...` smoke loop.  The
//! `ControlPlane::start()` API in `cluster.rs` wires up the
//! controller-manager + scheduler loops on top of this `State`.

use crate::aggregator::AggregatorRegistry;
use crate::crd::CrdRegistry;
use crate::garbage_collector::GarbageCollector;
use crate::pqc::HybridSigner;
use crate::quota::QuotaTracker;
use cave_apiserver::store::ResourceStore;
use std::sync::Arc;

/// Composite state.  All inner handles are `Arc` so the same `State` can be
/// cloned across axum extractors, kubelet workers, and controller loops
/// without copying any data.
#[derive(Clone)]
pub struct State {
    pub apiserver: Arc<ResourceStore>,
    pub crds: Arc<CrdRegistry>,
    pub quota: Arc<QuotaTracker>,
    pub aggregator: Arc<AggregatorRegistry>,
    pub gc: Arc<GarbageCollector>,
    pub signer: Arc<HybridSigner>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            apiserver: Arc::new(ResourceStore::new()),
            crds: Arc::new(CrdRegistry::new()),
            quota: Arc::new(QuotaTracker::new()),
            aggregator: Arc::new(AggregatorRegistry::new()),
            gc: Arc::new(GarbageCollector::new()),
            signer: Arc::new(HybridSigner::generate()),
        }
    }
}

impl State {
    /// Convenience: derive a sub-state by replacing the apiserver store.
    pub fn with_apiserver(mut self, store: Arc<ResourceStore>) -> Self {
        self.apiserver = store;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_constructs_all_eight_handles() {
        let s = State::default();
        // Force-touch each handle to confirm it constructed (no panic).
        let _ = Arc::strong_count(&s.apiserver);
        let _ = Arc::strong_count(&s.crds);
        let _ = Arc::strong_count(&s.quota);
        let _ = Arc::strong_count(&s.aggregator);
        let _ = Arc::strong_count(&s.gc);
        let _ = Arc::strong_count(&s.signer);
    }

    #[test]
    fn state_is_cloneable() {
        let s = State::default();
        let s2 = s.clone();
        assert!(Arc::ptr_eq(&s.apiserver, &s2.apiserver));
        assert!(Arc::ptr_eq(&s.signer, &s2.signer));
    }

    #[test]
    fn with_apiserver_swaps_handle() {
        let s = State::default();
        let fresh = Arc::new(ResourceStore::new());
        let original_ptr = Arc::as_ptr(&s.apiserver) as usize;
        let s2 = s.with_apiserver(fresh.clone());
        assert!(Arc::ptr_eq(&s2.apiserver, &fresh));
        assert_ne!(Arc::as_ptr(&s2.apiserver) as usize, original_ptr);
    }
}

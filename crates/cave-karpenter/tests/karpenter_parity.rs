// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests vs. upstream kubernetes-sigs/karpenter v1.12.0.
//!
//! All tests are `#[cfg(feature = "live-integration")]` until the corresponding upstream behaviour is
//! reimplemented. They exist so the compatibility surface is enumerated rather
//! than silently missing.

use cave_karpenter::*;

#[test]
#[cfg(feature = "live-integration")]
fn parity_nodepool_validation_rejects_empty_requirements() {
    // upstream: pkg/apis/v1/nodepool_validation.go
    // expectation: NodePool with no requirements is rejected at admission.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_nodeclaim_launch_creates_machine_via_provider() {
    // upstream: pkg/controllers/nodeclaim/lifecycle/launch.go
    // expectation: a non-empty NodeClaim triggers provider.Create() and records
    // providerID in NodeClaimStatus.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_consolidation_replaces_underutilised_nodes() {
    // upstream: pkg/controllers/disruption/consolidation.go
    // expectation: when a node is underutilised and a smaller fits, consolidation
    // emits a NodeClaim termination + replacement claim.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_drift_detects_nodeclass_change() {
    // upstream: pkg/controllers/disruption/drift.go
    // expectation: NodeClass spec mutation marks existing NodeClaims as Drifted=true.
    unimplemented!()
}

#[test]
#[cfg(feature = "live-integration")]
fn parity_scheduler_respects_resource_requests() {
    // upstream: pkg/controllers/provisioning/scheduling/scheduler.go
    // expectation: pod with cpu/memory requests larger than any pool's offering
    // is left pending (no NodeClaim created).
    unimplemented!()
}

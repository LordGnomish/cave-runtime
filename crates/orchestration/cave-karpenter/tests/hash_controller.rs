// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of the NodePool hash controller in
// pkg/controllers/nodepool/hash/controller.go from kubernetes-sigs/karpenter
// v1.12.1 (sha ed490e8). The controller stamps NodePool.Hash() onto the pool's
// `karpenter.sh/nodepool-hash` annotation at the current HashVersion ("v3"),
// then propagates the hash to owned NodeClaims that do not yet carry one
// (initial sync / version migration). Real drift — a claim that carries a hash
// differing from the pool's recomputed hash — is left for the disruption
// controller to act on.
//
// cave models the per-object annotation as NodePool.template_hash /
// NodeClaim.template_hash, so the port operates over those fields.

use cave_karpenter::disruption::drift_candidates;
use cave_karpenter::hash::nodepool_hash;
use cave_karpenter::hash_controller::{
    nodepool_hash_drifted, reconcile_hashes, stamp_nodepool_hash, NODEPOOL_HASH_ANNOTATION,
    NODEPOOL_HASH_VERSION, NODEPOOL_HASH_VERSION_ANNOTATION,
};
use cave_karpenter::models::{NodeClaim, NodePool, Requirement, RequirementOperator};

fn pool_with_req(name: &str, key: &str) -> NodePool {
    let mut p = NodePool::default();
    p.name = name.into();
    p.template.spec.requirements.push(Requirement {
        key: key.into(),
        operator: RequirementOperator::Exists,
        values: vec![],
        min_values: None,
    });
    p
}

// ---- constants ---------------------------------------------------------------

#[test]
fn hash_annotation_constants() {
    assert_eq!(NODEPOOL_HASH_ANNOTATION, "karpenter.sh/nodepool-hash");
    assert_eq!(
        NODEPOOL_HASH_VERSION_ANNOTATION,
        "karpenter.sh/nodepool-hash-version"
    );
    assert_eq!(NODEPOOL_HASH_VERSION, "v3");
}

// ---- stamp_nodepool_hash -----------------------------------------------------

#[test]
fn stamp_sets_template_hash_to_nodepool_hash() {
    let mut p = pool_with_req("p", "k");
    assert!(p.template_hash.is_none());
    stamp_nodepool_hash(&mut p);
    assert_eq!(p.template_hash.as_deref(), Some(nodepool_hash(&p).as_str()));
}

#[test]
fn stamp_is_idempotent() {
    let mut p = pool_with_req("p", "k");
    stamp_nodepool_hash(&mut p);
    let first = p.template_hash.clone();
    stamp_nodepool_hash(&mut p);
    assert_eq!(p.template_hash, first);
}

#[test]
fn stamp_changes_after_spec_mutation() {
    let mut p = pool_with_req("p", "k");
    stamp_nodepool_hash(&mut p);
    let before = p.template_hash.clone();
    p.template.spec.requirements.push(Requirement {
        key: "extra".into(),
        operator: RequirementOperator::Exists,
        values: vec![],
        min_values: None,
    });
    stamp_nodepool_hash(&mut p);
    assert_ne!(p.template_hash, before);
}

// ---- nodepool_hash_drifted ---------------------------------------------------

#[test]
fn drifted_when_claim_hash_differs_from_recomputed_pool_hash() {
    let p = pool_with_req("p", "k");
    let mut claim = NodeClaim::default();
    claim.pool_name = Some("p".into());
    claim.template_hash = Some("stale-hash".into());
    assert!(nodepool_hash_drifted(&claim, &p));
}

#[test]
fn not_drifted_when_claim_hash_matches() {
    let p = pool_with_req("p", "k");
    let mut claim = NodeClaim::default();
    claim.pool_name = Some("p".into());
    claim.template_hash = Some(nodepool_hash(&p));
    assert!(!nodepool_hash_drifted(&claim, &p));
}

#[test]
fn not_drifted_when_claim_has_no_hash() {
    // A claim with no recorded hash cannot be "drifted" — it needs an initial
    // stamp, not disruption.
    let p = pool_with_req("p", "k");
    let claim = NodeClaim::default();
    assert!(!nodepool_hash_drifted(&claim, &p));
}

// ---- reconcile_hashes --------------------------------------------------------

#[test]
fn reconcile_stamps_pools_and_syncs_unstamped_claims() {
    let mut pools = vec![pool_with_req("p", "k")];
    let mut claim = NodeClaim::default();
    claim.pool_name = Some("p".into());
    let mut claims = vec![claim];

    reconcile_hashes(&mut pools, &mut claims);

    let expected = nodepool_hash(&pools[0]);
    assert_eq!(pools[0].template_hash.as_deref(), Some(expected.as_str()));
    // The unstamped claim picks up the pool's hash → no drift.
    assert_eq!(claims[0].template_hash.as_deref(), Some(expected.as_str()));
    assert!(drift_candidates(&claims, &pools).is_empty());
}

#[test]
fn reconcile_leaves_existing_claim_hash_for_drift_detection() {
    let mut pools = vec![pool_with_req("p", "k")];
    let mut claim = NodeClaim::default();
    claim.pool_name = Some("p".into());
    claim.template_hash = Some("old-launch-hash".into());
    let mut claims = vec![claim];

    reconcile_hashes(&mut pools, &mut claims);

    // Existing hash is NOT overwritten...
    assert_eq!(claims[0].template_hash.as_deref(), Some("old-launch-hash"));
    // ...so the disruption controller can see the drift.
    let drifted = drift_candidates(&claims, &pools);
    assert_eq!(drifted.len(), 1);
}

// ---- end-to-end: stamp → schedule → mutate → drift ---------------------------

#[test]
fn pool_spec_change_after_launch_produces_drift() {
    let mut pools = vec![pool_with_req("p", "k")];
    // Launch-time: claim copies the freshly stamped pool hash.
    stamp_nodepool_hash(&mut pools[0]);
    let mut claim = NodeClaim::default();
    claim.pool_name = Some("p".into());
    claim.template_hash = pools[0].template_hash.clone();
    let mut claims = vec![claim];

    // No drift yet.
    assert!(drift_candidates(&claims, &pools).is_empty());

    // Operator edits the pool spec; the controller re-stamps the pool.
    pools[0].template.spec.requirements.push(Requirement {
        key: "new".into(),
        operator: RequirementOperator::Exists,
        values: vec![],
        min_values: None,
    });
    reconcile_hashes(&mut pools, &mut claims);

    // The claim's old hash now differs from the pool → drift.
    let drifted = drift_candidates(&claims, &pools);
    assert_eq!(drifted.len(), 1);
}

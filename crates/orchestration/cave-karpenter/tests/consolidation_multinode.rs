// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-node + single-node consolidation — faithful port of
//! kubernetes-sigs/karpenter v1.4.0
//!   pkg/controllers/disruption/consolidation.go            (computeConsolidation)
//!   pkg/controllers/disruption/multinodeconsolidation.go   (firstNConsolidationOption)
//!   pkg/controllers/disruption/singlenodeconsolidation.go  (ComputeCommand)
//!   pkg/controllers/disruption/types.go                    (Command / Decision)
//!
//! The headline behaviour: take a set of underutilised nodes, simulate
//! rescheduling their pods, and emit a Command that merges them into
//! *fewer* nodes — either a Delete (pods fit the remaining cluster) or a
//! Replace (one cheaper node replaces several).

use cave_karpenter::batcher::PodSpec;
use cave_karpenter::consolidation::{
    Command, Decision, InstanceOffering, NodeCapacity, compute_consolidation,
    multi_node_consolidation, simulate_scheduling, single_node_consolidation,
    ConsolidationCandidate,
};
use std::collections::HashMap;

fn pod(name: &str, cpu: u32, mem: u32) -> PodSpec {
    PodSpec::with_resources(name, cpu, mem)
}

fn offering(name: &str, cpu: u32, mem: u32, price: f64) -> InstanceOffering {
    InstanceOffering {
        name: name.into(),
        cpu_millis: cpu,
        memory_mib: mem,
        zone: "z1".into(),
        price,
    }
}

/// A candidate node: a `pool`, the type it currently runs on (with its
/// price), and the pods that would need rescheduling if it is removed.
fn candidate(name: &str, pool: &str, price: f64, pods: Vec<PodSpec>) -> ConsolidationCandidate {
    ConsolidationCandidate {
        claim_name: name.into(),
        nodepool: pool.into(),
        instance_type: name.into(),
        zone: "z1".into(),
        price,
        disruption_cost: pods.iter().map(|p| p.cpu_millis as f64).sum(),
        reschedulable_pods: pods,
    }
}

fn no_budget_limit() -> HashMap<String, i32> {
    let mut m = HashMap::new();
    m.insert("default".to_string(), 100);
    m
}

// ── Decision classification (types.go) ──────────────────────────────────────

#[test]
fn decision_maps_candidates_and_replacements() {
    assert_eq!(Command::default().decision(), Decision::NoOp);
}

// ── computeConsolidation: Delete when pods fit the remaining cluster ─────────

#[test]
fn consolidation_deletes_when_pods_fit_remaining_capacity() {
    // One under-utilised node holding a tiny pod; the rest of the cluster
    // has room → the node can simply be removed (Delete, no replacement).
    let cands = vec![candidate("n1", "default", 1.0, vec![pod("p", 200, 256)])];
    let remaining = vec![NodeCapacity {
        free_cpu_millis: 4000,
        free_memory_mib: 8192,
    }];
    let cmd = compute_consolidation(&cands, &remaining, &[]);
    assert_eq!(cmd.decision(), Decision::Delete);
    assert_eq!(cmd.candidates().len(), 1);
    assert!(cmd.replacements().is_empty());
}

// ── computeConsolidation: Replace with a single cheaper node ─────────────────

#[test]
fn consolidation_replaces_with_single_cheaper_node() {
    // Two pods, no remaining capacity, but one cheap big offering fits both
    // and is cheaper than the candidate's current price → Replace.
    let cands = vec![candidate(
        "expensive",
        "default",
        10.0,
        vec![pod("a", 500, 512), pod("b", 500, 512)],
    )];
    let offerings = vec![offering("cheap-big", 2000, 4096, 4.0)];
    let cmd = compute_consolidation(&cands, &[], &offerings);
    assert_eq!(cmd.decision(), Decision::Replace);
    assert_eq!(cmd.replacements().len(), 1);
    assert_eq!(cmd.replacements()[0].offering.name, "cheap-big");
}

#[test]
fn consolidation_noop_when_replacement_not_cheaper() {
    let cands = vec![candidate("n1", "default", 3.0, vec![pod("a", 500, 512)])];
    // Only offering is *more* expensive than the current node → keep it.
    let offerings = vec![offering("pricey", 2000, 4096, 9.0)];
    let cmd = compute_consolidation(&cands, &[], &offerings);
    assert_eq!(cmd.decision(), Decision::NoOp);
}

#[test]
fn consolidation_noop_when_pods_do_not_fit_anywhere() {
    let cands = vec![candidate("n1", "default", 5.0, vec![pod("huge", 9000, 9999)])];
    let offerings = vec![offering("small", 1000, 1024, 1.0)];
    let cmd = compute_consolidation(&cands, &[], &offerings);
    assert_eq!(cmd.decision(), Decision::NoOp);
}

// ── multiNodeConsolidation: merge several nodes into ONE ─────────────────────

#[test]
fn multi_node_consolidation_merges_three_into_one() {
    // Three small under-utilised nodes, each $5, holding one pod apiece.
    // A single $9 offering holds all three pods. The faithful firstN binary
    // search climbs only through consolidatable prefixes, so each prefix is
    // also favourable ($10 > $9 for two, $15 > $9 for three) → 3 nodes → 1.
    let cands = vec![
        candidate("n1", "default", 5.0, vec![pod("p1", 500, 512)]),
        candidate("n2", "default", 5.0, vec![pod("p2", 500, 512)]),
        candidate("n3", "default", 5.0, vec![pod("p3", 500, 512)]),
    ];
    let offerings = vec![offering("merged", 4000, 8192, 9.0)];
    let cmd = multi_node_consolidation(&cands, no_budget_limit(), &[], &offerings);
    assert_eq!(cmd.decision(), Decision::Replace);
    assert_eq!(cmd.candidates().len(), 3, "all three nodes consolidated");
    assert_eq!(cmd.replacements().len(), 1, "into a single replacement node");
}

#[test]
fn multi_node_consolidation_deletes_three_when_cluster_has_room() {
    let cands = vec![
        candidate("n1", "default", 4.0, vec![pod("p1", 200, 256)]),
        candidate("n2", "default", 4.0, vec![pod("p2", 200, 256)]),
        candidate("n3", "default", 4.0, vec![pod("p3", 200, 256)]),
    ];
    // Plenty of spare capacity on the surviving nodes.
    let remaining = vec![NodeCapacity {
        free_cpu_millis: 8000,
        free_memory_mib: 16384,
    }];
    let cmd = multi_node_consolidation(&cands, no_budget_limit(), &remaining, &[]);
    assert_eq!(cmd.decision(), Decision::Delete);
    assert_eq!(cmd.candidates().len(), 3);
    assert!(cmd.replacements().is_empty());
}

#[test]
fn multi_node_binary_search_picks_largest_consolidatable_batch() {
    // n1,n2 fit on remaining capacity; n3 holds a pod too big to reschedule.
    // The binary search must consolidate the largest prefix [n1,n2] and
    // stop before the un-reschedulable n3.
    let cands = vec![
        candidate("n1", "default", 4.0, vec![pod("p1", 200, 256)]),
        candidate("n2", "default", 4.0, vec![pod("p2", 200, 256)]),
        candidate("n3", "default", 4.0, vec![pod("huge", 9000, 9000)]),
    ];
    let remaining = vec![NodeCapacity {
        free_cpu_millis: 1000,
        free_memory_mib: 2048,
    }];
    let cmd = multi_node_consolidation(&cands, no_budget_limit(), &remaining, &[]);
    assert_eq!(cmd.decision(), Decision::Delete);
    assert_eq!(cmd.candidates().len(), 2, "only n1 and n2 consolidate");
}

#[test]
fn multi_node_consolidation_needs_at_least_two_candidates() {
    let cands = vec![candidate("n1", "default", 4.0, vec![pod("p1", 200, 256)])];
    let remaining = vec![NodeCapacity {
        free_cpu_millis: 8000,
        free_memory_mib: 16384,
    }];
    // Single candidate → multi-node returns NoOp (single-node handles it).
    let cmd = multi_node_consolidation(&cands, no_budget_limit(), &remaining, &[]);
    assert_eq!(cmd.decision(), Decision::NoOp);
}

#[test]
fn multi_node_consolidation_respects_zero_budget() {
    let cands = vec![
        candidate("n1", "default", 4.0, vec![pod("p1", 200, 256)]),
        candidate("n2", "default", 4.0, vec![pod("p2", 200, 256)]),
    ];
    let remaining = vec![NodeCapacity {
        free_cpu_millis: 8000,
        free_memory_mib: 16384,
    }];
    let mut budget = HashMap::new();
    budget.insert("default".to_string(), 0);
    let cmd = multi_node_consolidation(&cands, budget, &remaining, &[]);
    assert_eq!(cmd.decision(), Decision::NoOp);
}

// ── singleNodeConsolidation ──────────────────────────────────────────────────

#[test]
fn single_node_consolidation_replaces_first_eligible() {
    let cands = vec![
        candidate("n1", "default", 8.0, vec![pod("p1", 500, 512)]),
        candidate("n2", "default", 8.0, vec![pod("p2", 500, 512)]),
    ];
    let offerings = vec![offering("cheaper", 1000, 1024, 3.0)];
    let cmd = single_node_consolidation(&cands, no_budget_limit(), &[], &offerings);
    assert_eq!(cmd.decision(), Decision::Replace);
    assert_eq!(cmd.candidates().len(), 1, "single-node touches one node");
}

#[test]
fn single_node_consolidation_skips_empty_then_deletes() {
    // n1 has no reschedulable pods (empty); n2 fits on remaining → delete n2.
    let cands = vec![
        candidate("n1", "default", 8.0, vec![]),
        candidate("n2", "default", 8.0, vec![pod("p2", 200, 256)]),
    ];
    let remaining = vec![NodeCapacity {
        free_cpu_millis: 4000,
        free_memory_mib: 4096,
    }];
    let cmd = single_node_consolidation(&cands, no_budget_limit(), &remaining, &[]);
    assert_eq!(cmd.decision(), Decision::Delete);
    assert_eq!(cmd.candidates()[0].claim_name, "n2");
}

// ── SimulateScheduling primitive ─────────────────────────────────────────────

#[test]
fn simulate_scheduling_prefers_remaining_capacity_over_new_nodes() {
    let pods = vec![pod("a", 300, 256), pod("b", 300, 256)];
    let remaining = vec![NodeCapacity {
        free_cpu_millis: 2000,
        free_memory_mib: 2048,
    }];
    let res = simulate_scheduling(&pods, &remaining, &[]);
    assert!(res.all_scheduled);
    assert!(
        res.new_node_claims.is_empty(),
        "pods absorbed by remaining capacity, no new nodes"
    );
}

#[test]
fn simulate_scheduling_opens_new_node_when_remaining_full() {
    let pods = vec![pod("a", 800, 512)];
    let offerings = vec![offering("new", 1000, 1024, 2.0)];
    let res = simulate_scheduling(&pods, &[], &offerings);
    assert!(res.all_scheduled);
    assert_eq!(res.new_node_claims.len(), 1);
    assert_eq!(res.new_node_claims[0].pods, vec!["a".to_string()]);
}

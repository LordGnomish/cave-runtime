// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Batch 3 (2026-05-14) — additional upstream test ports beyond
//! `upstream_port.rs` (batch1, 2026-05-13).
//!
//! Batch1 covered imagelocality + tainttoleration filter +
//! nodeaffinity In/NotIn + nodename + nodeunschedulable +
//! noderesources/least_allocated basics.
//!
//! Batch3 expands into nodeaffinity Gt/Lt + podaffinity (anti) +
//! topology spread filter/score + NodeResourcesFit MostAllocated +
//! extended-resource quota.
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   * pkg/scheduler/framework/plugins/nodeaffinity/node_affinity_test.go
//!   * pkg/scheduler/framework/plugins/interpodaffinity/{filter,score}_test.go
//!   * pkg/scheduler/framework/plugins/podtopologyspread/{filtering,scoring}_test.go
//!   * pkg/scheduler/framework/plugins/noderesources/{fit,most_allocated}_test.go

use cave_scheduler::framework::{
    ClusterSnapshot, Code, FilterPlugin, MAX_NODE_SCORE, NodeAffinitySpec, NodeSelectorOp,
    NodeSelectorRequirement, NodeSelectorTerm, Pod, PodAffinityTerm, ScorePlugin,
    TopologySpreadConstraint, UnsatisfiableAction,
};
use cave_scheduler::models::{
    Node, NodeStatus, ResourceCapacity, ResourceRequest, Taint, TaintEffect, Toleration,
};
use cave_scheduler::noderesources::{
    ExtendedResourcesState, NodeResourcesFit, NodeResourcesFitArgs, ScoringStrategyType,
};
use cave_scheduler::plugins::{InterPodAffinity, NodeAffinity, TaintToleration};
use cave_scheduler::topology::PodTopologySpread;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn n_named(name: &str, labels: &[(&str, &str)]) -> Node {
    let mut node = Node {
        name: name.into(),
        uid: Uuid::new_v4(),
        status: NodeStatus::Ready,
        capacity: ResourceCapacity {
            cpu_millicores: 4000,
            memory_bytes: 8_000_000_000,
            pods: 110,
            ephemeral_storage_bytes: 0,
        },
        allocatable: ResourceCapacity {
            cpu_millicores: 4000,
            memory_bytes: 8_000_000_000,
            pods: 110,
            ephemeral_storage_bytes: 0,
        },
        allocated: ResourceCapacity::default(),
        labels: HashMap::new(),
        taints: vec![],
        conditions: vec![],
        registered_at: Utc::now(),
        last_heartbeat: Utc::now(),
    };
    for (k, v) in labels {
        node.labels.insert((*k).to_string(), (*v).to_string());
    }
    node
}

fn snap(nodes: Vec<Node>) -> ClusterSnapshot {
    ClusterSnapshot {
        nodes,
        pods_by_node: HashMap::new(),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/nodeaffinity/node_affinity_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestNodeAffinity / `Gt_operator_passes_when_numeric_node_value_greater`.
#[test]
fn upstream_node_affinity_gt_operator_passes_when_node_value_greater() {
    let node = n_named("a", &[("cores", "16")]);
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.node_affinity = Some(NodeAffinitySpec {
        required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement {
                key: "cores".into(),
                operator: NodeSelectorOp::Gt,
                values: vec!["8".into()],
            }],
        }],
        ..Default::default()
    });
    assert!(NodeAffinity
        .filter(&pod, &node, &snap(vec![]))
        .is_success());
}

/// Upstream: TestNodeAffinity / `Lt_operator_blocks_when_node_value_greater_or_equal`.
#[test]
fn upstream_node_affinity_lt_operator_blocks_when_node_value_not_less() {
    let node = n_named("a", &[("cores", "16")]);
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.node_affinity = Some(NodeAffinitySpec {
        required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement {
                key: "cores".into(),
                operator: NodeSelectorOp::Lt,
                values: vec!["8".into()],
            }],
        }],
        ..Default::default()
    });
    let status = NodeAffinity.filter(&pod, &node, &snap(vec![]));
    assert_eq!(status.code, Code::Unschedulable);
}

/// Upstream: TestNodeAffinity / `Exists_operator_matches_any_value`.
/// `Exists` doesn't care about `values`, only that the key is present.
#[test]
fn upstream_node_affinity_exists_operator_matches_any_value() {
    let labeled = n_named("a", &[("disk", "ssd")]);
    let bare = n_named("b", &[]);
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.node_affinity = Some(NodeAffinitySpec {
        required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement {
                key: "disk".into(),
                operator: NodeSelectorOp::Exists,
                values: vec![],
            }],
        }],
        ..Default::default()
    });
    assert!(NodeAffinity
        .filter(&pod, &labeled, &snap(vec![]))
        .is_success());
    let st = NodeAffinity.filter(&pod, &bare, &snap(vec![]));
    assert_eq!(st.code, Code::Unschedulable);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/interpodaffinity/filter_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestInterPodAffinity / `pod_affinity_requires_matching_pod_in_topology_domain`.
/// PodAffinity wants ≥1 selector-matching pod on a node sharing the same
/// topology key value as the candidate; without it → Unschedulable.
#[test]
fn upstream_interpod_affinity_requires_peer_in_same_topology_domain() {
    let target = n_named("target", &[("zone", "us-east-1a")]);
    let other_zone = n_named("other", &[("zone", "us-east-1b")]);
    let mut snap = snap(vec![target.clone(), other_zone.clone()]);

    // A peer pod with label app=web on the OTHER zone — should NOT satisfy
    // affinity for `target` (different zone).
    let mut peer = Pod::new("t", "ns", "peer");
    peer.spec.node_selector.insert("app".into(), "web".into());
    snap.pods_by_node.insert("other".into(), vec![peer]);

    let mut affinity_term = PodAffinityTerm::default();
    affinity_term
        .label_selector
        .insert("app".into(), "web".into());
    affinity_term.topology_key = "zone".into();

    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.pod_affinity.push(affinity_term);

    let status = InterPodAffinity.filter(&pod, &target, &snap);
    assert_eq!(status.code, Code::Unschedulable);
}

/// Upstream: TestInterPodAffinity / `anti_affinity_blocks_co_location`.
/// PodAntiAffinity: a single matching peer on the same topology value
/// blocks scheduling.
#[test]
fn upstream_interpod_anti_affinity_blocks_when_peer_already_in_domain() {
    let target = n_named("target", &[("zone", "us-east-1a")]);
    let mut snap = snap(vec![target.clone()]);
    let mut peer = Pod::new("t", "ns", "peer");
    peer.spec.node_selector.insert("app".into(), "web".into());
    snap.pods_by_node.insert("target".into(), vec![peer]);

    let mut anti = PodAffinityTerm::default();
    anti.label_selector.insert("app".into(), "web".into());
    anti.topology_key = "zone".into();

    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.node_selector.insert("app".into(), "web".into());
    pod.spec.pod_anti_affinity.push(anti);

    let status = InterPodAffinity.filter(&pod, &target, &snap);
    assert_eq!(status.code, Code::Unschedulable);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/tainttoleration/taint_toleration_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestTaintTolerationFilter / `NoExecute_taint_blocked_when_not_tolerated`.
/// NoExecute is filter-blocking (not just runtime-eviction-triggering).
#[test]
fn upstream_taint_toleration_no_execute_blocks_filter_when_not_tolerated() {
    let mut node = n_named("a", &[]);
    node.taints.push(Taint {
        key: "drain".into(),
        value: None,
        effect: TaintEffect::NoExecute,
    });
    let pod = Pod::new("t", "ns", "p");
    let status = TaintToleration.filter(&pod, &node, &snap(vec![]));
    assert_eq!(status.code, Code::Unschedulable);
}

/// Upstream: TestTaintTolerationFilter / `PreferNoSchedule_is_advisory_at_filter_stage`.
/// PreferNoSchedule MUST NOT cause an Unschedulable verdict.
#[test]
fn upstream_taint_toleration_prefer_no_schedule_does_not_block_filter() {
    let mut node = n_named("a", &[]);
    node.taints.push(Taint {
        key: "soft".into(),
        value: None,
        effect: TaintEffect::PreferNoSchedule,
    });
    let pod = Pod::new("t", "ns", "p");
    let status = TaintToleration.filter(&pod, &node, &snap(vec![]));
    assert!(
        status.is_success(),
        "PreferNoSchedule is advisory only at the filter stage; got {status:?}"
    );
}

/// Upstream: TestTaintTolerationFilter / `Equal_value_mismatch_blocks_filter`.
#[test]
fn upstream_taint_toleration_equal_value_mismatch_does_not_tolerate() {
    let mut node = n_named("a", &[]);
    node.taints.push(Taint {
        key: "dedicated".into(),
        value: Some("gpu".into()),
        effect: TaintEffect::NoSchedule,
    });
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.tolerations.push(Toleration {
        key: Some("dedicated".into()),
        operator: "Equal".into(),
        value: Some("cpu".into()), // wrong value
        effect: Some(TaintEffect::NoSchedule),
    });
    let status = TaintToleration.filter(&pod, &node, &snap(vec![]));
    assert_eq!(status.code, Code::Unschedulable);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/podtopologyspread/filtering_test.go
// ────────────────────────────────────────────────────────────────────────────

fn topology_spread(max_skew: i32, action: UnsatisfiableAction) -> TopologySpreadConstraint {
    let mut sel = HashMap::new();
    sel.insert("app".into(), "web".into());
    TopologySpreadConstraint {
        max_skew,
        topology_key: "zone".into(),
        when_unsatisfiable: action,
        label_selector: sel,
        min_domains: None,
        ..Default::default()
    }
}

/// Upstream: TestPodTopologySpread / `do_not_schedule_blocks_when_skew_exceeded`.
#[test]
fn upstream_topology_spread_do_not_schedule_blocks_when_skew_exceeded() {
    let a = n_named("a", &[("zone", "z1")]);
    let b = n_named("b", &[("zone", "z2")]);
    let mut s = snap(vec![a.clone(), b.clone()]);
    let mut peer1 = Pod::new("t", "ns", "p1");
    peer1.spec.node_selector.insert("app".into(), "web".into());
    let mut peer2 = Pod::new("t", "ns", "p2");
    peer2.spec.node_selector.insert("app".into(), "web".into());
    s.pods_by_node.insert("a".into(), vec![peer1, peer2]);

    let mut pod = Pod::new("t", "ns", "newp");
    pod.spec.node_selector.insert("app".into(), "web".into());
    pod.spec
        .topology_spread
        .push(topology_spread(1, UnsatisfiableAction::DoNotSchedule));

    // Placing on z1 → skew 3-0 = 3 > maxSkew 1.
    let status = PodTopologySpread.filter(&pod, &a, &s);
    assert_eq!(status.code, Code::Unschedulable);
    // Placing on z2 → skew 2-1 = 1 ≤ 1 → OK.
    assert!(PodTopologySpread.filter(&pod, &b, &s).is_success());
}

/// Upstream: TestPodTopologySpread / `schedule_anyway_does_not_filter`.
/// ScheduleAnyway is a soft constraint — Filter always returns Success.
#[test]
fn upstream_topology_spread_schedule_anyway_filter_always_passes() {
    let a = n_named("a", &[("zone", "z1")]);
    let b = n_named("b", &[("zone", "z2")]);
    let mut s = snap(vec![a.clone(), b.clone()]);
    let mut peer1 = Pod::new("t", "ns", "p1");
    peer1.spec.node_selector.insert("app".into(), "web".into());
    let mut peer2 = Pod::new("t", "ns", "p2");
    peer2.spec.node_selector.insert("app".into(), "web".into());
    s.pods_by_node.insert("a".into(), vec![peer1, peer2]);

    let mut pod = Pod::new("t", "ns", "newp");
    pod.spec.node_selector.insert("app".into(), "web".into());
    pod.spec
        .topology_spread
        .push(topology_spread(1, UnsatisfiableAction::ScheduleAnyway));

    assert!(PodTopologySpread.filter(&pod, &a, &s).is_success());
    assert!(PodTopologySpread.filter(&pod, &b, &s).is_success());
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/noderesources/most_allocated_test.go
// + extended_resources_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestMostAllocated / `picks_higher_utilisation_node`.
/// MostAllocated bin-packs — the node with MORE used resources wins.
#[test]
fn upstream_node_resources_most_allocated_prefers_higher_utilisation() {
    let mut hot = n_named("hot", &[]);
    hot.allocated.cpu_millicores = 3000;
    hot.allocated.memory_bytes = 6_000_000_000;
    let cold = n_named("cold", &[]); // allocated = 0
    let pod = Pod::new("t", "ns", "p");
    let plugin = NodeResourcesFit::new(NodeResourcesFitArgs {
        scoring_strategy: ScoringStrategyType::MostAllocated,
        ..Default::default()
    });
    let s = snap(vec![]);
    let hot_score = plugin.score(&pod, &hot, &s);
    let cold_score = plugin.score(&pod, &cold, &s);
    assert!(
        hot_score > cold_score,
        "MostAllocated should prefer the more-utilised node; hot={hot_score} cold={cold_score}"
    );
}

/// Upstream: TestNodeResourcesFit_ExtendedResource / `extended_resource_insufficient_blocks`.
/// Filter denies when an extended resource (e.g. nvidia.com/gpu) is
/// requested beyond the node's free capacity.
#[test]
fn upstream_node_resources_fit_blocks_when_extended_resource_insufficient() {
    let node = n_named("gpu-1", &[]);
    let mut extended = ExtendedResourcesState::default();
    extended.set_capacity("gpu-1", "nvidia.com/gpu", 4);
    extended.set_allocated("gpu-1", "nvidia.com/gpu", 3);
    let plugin = NodeResourcesFit::new(NodeResourcesFitArgs::default())
        .with_extended(extended);

    let mut pod = Pod::new("t", "ns", "p");
    pod.spec
        .resources
        .extended
        .insert("nvidia.com/gpu".into(), 2);
    let status = plugin.filter(&pod, &node, &snap(vec![]));
    assert_eq!(status.code, Code::Unschedulable);
    // With request that fits, it passes.
    let mut pod2 = Pod::new("t", "ns", "p2");
    pod2.spec
        .resources
        .extended
        .insert("nvidia.com/gpu".into(), 1);
    assert!(plugin.filter(&pod2, &node, &snap(vec![])).is_success());
}

/// Upstream: TestNodeResourcesFit / `pod_capacity_exhausted_blocks`.
/// Distinct from CPU/memory exhaustion — pod-count cap is its own check.
#[test]
fn upstream_node_resources_fit_blocks_when_pod_count_capacity_exhausted() {
    let mut node = n_named("a", &[]);
    node.allocated.pods = node.allocatable.pods; // saturated
    let pod = Pod::new("t", "ns", "p");
    let plugin = NodeResourcesFit::new(NodeResourcesFitArgs::default());
    let status = plugin.filter(&pod, &node, &snap(vec![]));
    assert_eq!(status.code, Code::Unschedulable);
}

/// Upstream: TestNodeResourcesFit / `ignored_resource_skips_filter_check`.
/// `ignored_resources` makes a resource invisible to the filter.
#[test]
fn upstream_node_resources_fit_ignored_resource_skips_filter() {
    let node = n_named("a", &[]);
    // Pod requests more memory than the node has, but memory is ignored.
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.resources = ResourceRequest {
        cpu_millicores: 100,
        memory_bytes: u64::MAX,
        ..Default::default()
    };
    let plugin = NodeResourcesFit::new(NodeResourcesFitArgs {
        ignored_resources: vec!["memory".into()],
        ..Default::default()
    });
    assert!(plugin.filter(&pod, &node, &snap(vec![])).is_success());
}

/// Upstream: TestNodeResourcesFit_LeastAllocated / `cold_node_outscores_hot_node`.
#[test]
fn upstream_node_resources_fit_least_allocated_prefers_cold_node() {
    let mut hot = n_named("hot", &[]);
    hot.allocated.cpu_millicores = 3500;
    hot.allocated.memory_bytes = 7_000_000_000;
    let cold = n_named("cold", &[]);
    let pod = Pod::new("t", "ns", "p");
    let plugin = NodeResourcesFit::new(NodeResourcesFitArgs::default()); // default = LeastAllocated
    let s = snap(vec![]);
    let cold_score = plugin.score(&pod, &cold, &s);
    let hot_score = plugin.score(&pod, &hot, &s);
    assert!(
        cold_score > hot_score,
        "LeastAllocated should prefer the colder node; cold={cold_score} hot={hot_score}"
    );
    assert!(cold_score <= MAX_NODE_SCORE);
}

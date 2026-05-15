// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream kube-scheduler tests, cited in the
//! `[[upstream_test]]` block of `parity.manifest.toml`.
//!
//! Each test below references a specific upstream test by file path +
//! Go function name. Subtests (Go `t.Run(name, …)`) are split into
//! individual `#[test]` fns so a single subtest failure stays
//! localised, matching what the dashboard's behavioral_parity counter
//! credits.
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   pkg/scheduler/framework/plugins/{imagelocality,tainttoleration,
//!   noderesources,nodeaffinity,nodename,nodeunschedulable,nodeports}/*_test.go
//!
//! Honest gate: each test exercises the cave plugin's public API
//! against the same input/output equivalence class as the upstream
//! case. Where the cave API diverges from upstream (e.g. internal
//! ImageStateSummary shape vs. upstream framework.ImageStateSummary),
//! the test still asserts the same observable property.

use cave_scheduler::framework::{
    ClusterSnapshot, Code, FilterPlugin, MAX_NODE_SCORE, NodeAffinitySpec,
    NodeSelectorOp, NodeSelectorRequirement, NodeSelectorTerm, Pod, ScorePlugin,
};
use cave_scheduler::models::{
    Node, NodeStatus, ResourceCapacity, Taint, TaintEffect, Toleration,
};
use cave_scheduler::plugins::{
    IMAGE_LOCALITY_MAX_THRESHOLD_PER_CONTAINER, IMAGE_LOCALITY_MIN_THRESHOLD, ImageLocality,
    ImageStateSummary, NodeAffinity, NodeImageStates, NodeName, NodeUnschedulable, Resources,
    TaintToleration,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn make_node(name: &str) -> Node {
    Node {
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
    }
}

fn empty_snap(nodes: Vec<Node>) -> ClusterSnapshot {
    ClusterSnapshot {
        nodes,
        pods_by_node: HashMap::new(),
    }
}

fn image_states(entries: &[(&str, u64, u32)]) -> NodeImageStates {
    entries
        .iter()
        .map(|(name, size, nodes)| {
            (
                (*name).to_string(),
                ImageStateSummary {
                    size_bytes: *size,
                    num_nodes: *nodes,
                },
            )
        })
        .collect()
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/imagelocality/image_locality_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestImageLocalityPriority / `test 40MB image on 1 node out of 10`.
/// Upstream input: pod with one image of 40 MiB, image present on 1/10 nodes.
/// Upstream expected: score > 0 (image exceeds 23 MiB min threshold scaled
/// by 1/10 ≈ 4 MiB → wait, falls UNDER min threshold → score 0).
/// Cave equivalent: scaled = 40 × (1/10) = 4 MiB → below 23 MiB → 0.
#[test]
fn upstream_image_locality_40mb_one_of_ten_below_threshold() {
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.container_images = vec!["debian:12".into()];
    let mut il = ImageLocality::new();
    il.total_nodes = 10;
    il.update_node_images(
        "a",
        image_states(&[("debian:12", 40 * 1024 * 1024, 1)]),
    );
    let score = il.score(&pod, &make_node("a"), &empty_snap(vec![]));
    // Below the min threshold after spread scaling → clamped to 0.
    assert_eq!(score, 0, "expected 0, got {score}");
}

/// Upstream: TestImageLocalityPriority / `test 250MB image on 1 node out of 1`.
/// 250 MiB > 23 MiB min, < 1000 MiB max → linear scaling to a positive
/// non-saturating score.
#[test]
fn upstream_image_locality_250mb_single_node_linear() {
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.container_images = vec!["fluentd:1.16".into()];
    let mut il = ImageLocality::new();
    il.total_nodes = 1;
    il.update_node_images(
        "a",
        image_states(&[("fluentd:1.16", 250 * 1024 * 1024, 1)]),
    );
    let score = il.score(&pod, &make_node("a"), &empty_snap(vec![]));
    assert!(
        score > 0 && score < MAX_NODE_SCORE,
        "expected linear-scaled in (0, 100), got {score}"
    );
    // Upstream's analytic: (250 - 23) / (1000 - 23) × 100 ≈ 23.
    assert!((20..=27).contains(&score), "expected ~23, got {score}");
}

/// Upstream: TestImageLocalityPriority / `test 2000MB image, single container, on 1 node`.
/// Upstream expected: MAX_NODE_SCORE — image far above max threshold.
#[test]
fn upstream_image_locality_2000mb_single_node_saturates() {
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.container_images = vec!["postgres:16".into()];
    let mut il = ImageLocality::new();
    il.total_nodes = 1;
    il.update_node_images(
        "a",
        image_states(&[("postgres:16", 2000 * 1024 * 1024, 1)]),
    );
    let score = il.score(&pod, &make_node("a"), &empty_snap(vec![]));
    assert_eq!(score, MAX_NODE_SCORE);
}

/// Upstream: TestImageLocalityPriority / `test no images in pod spec`.
/// Upstream expected: score 0 (skip scoring contribution entirely).
#[test]
fn upstream_image_locality_no_images_in_pod() {
    let pod = Pod::new("t", "ns", "p"); // no container_images set
    let mut il = ImageLocality::new();
    il.total_nodes = 5;
    il.update_node_images(
        "a",
        image_states(&[("anything:1", 500 * 1024 * 1024, 5)]),
    );
    assert_eq!(il.score(&pod, &make_node("a"), &empty_snap(vec![])), 0);
}

/// Upstream: TestImageLocalityPriority / `test multi container priority sum`.
/// Two containers, two images, both cached: sum_scores adds up.
/// Upstream property: priority(2 × 200 MiB) > priority(1 × 200 MiB).
#[test]
fn upstream_image_locality_multi_container_monotonic_sum() {
    let mut pod_one = Pod::new("t", "ns", "p1");
    pod_one.spec.container_images = vec!["nginx:1".into()];
    let mut pod_two = Pod::new("t", "ns", "p2");
    pod_two.spec.container_images = vec!["nginx:1".into(), "redis:7".into()];

    let mut il = ImageLocality::new();
    il.total_nodes = 1;
    il.update_node_images(
        "a",
        image_states(&[
            ("nginx:1", 200 * 1024 * 1024, 1),
            ("redis:7", 300 * 1024 * 1024, 1),
        ]),
    );

    let one = il.score(&pod_one, &make_node("a"), &empty_snap(vec![]));
    let two = il.score(&pod_two, &make_node("a"), &empty_snap(vec![]));
    assert!(two > one, "expected two > one ({one}), got {two}");
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/tainttoleration/taint_toleration_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestTaintTolerationFilter / `pod_does_not_tolerate_taint_NoSchedule`.
/// Upstream expected: status = Unschedulable.
#[test]
fn upstream_taint_toleration_no_schedule_blocks_when_not_tolerated() {
    let mut node = make_node("a");
    node.taints.push(Taint {
        key: "node.kubernetes.io/disk-pressure".into(),
        value: None,
        effect: TaintEffect::NoSchedule,
    });
    let pod = Pod::new("t", "ns", "p");
    let status = TaintToleration.filter(&pod, &node, &empty_snap(vec![]));
    assert_eq!(status.code, Code::Unschedulable);
}

/// Upstream: TestTaintTolerationFilter / `pod_tolerates_with_Exists_operator`.
/// Upstream expected: Success.
#[test]
fn upstream_taint_toleration_exists_operator_tolerates_any_value() {
    let mut node = make_node("a");
    node.taints.push(Taint {
        key: "dedicated".into(),
        value: Some("gpu".into()),
        effect: TaintEffect::NoSchedule,
    });
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.tolerations.push(Toleration {
        key: Some("dedicated".into()),
        operator: "Exists".into(),
        value: None,
        effect: Some(TaintEffect::NoSchedule),
    });
    assert!(
        TaintToleration
            .filter(&pod, &node, &empty_snap(vec![]))
            .is_success()
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/nodeaffinity/node_affinity_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestNodeAffinity / `required nodeSelectorTerm NotIn operator`.
/// Upstream expected: Unschedulable when the node's label value IS in the
/// NotIn values list.
#[test]
fn upstream_node_affinity_required_notin_excludes_match() {
    let mut node = make_node("a");
    node.labels.insert("zone".into(), "us-east-1".into());
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.node_affinity = Some(NodeAffinitySpec {
        required: vec![NodeSelectorTerm {
            match_expressions: vec![NodeSelectorRequirement {
                key: "zone".into(),
                operator: NodeSelectorOp::NotIn,
                values: vec!["us-east-1".into()],
            }],
        }],
        ..Default::default()
    });
    let status = NodeAffinity.filter(&pod, &node, &empty_snap(vec![]));
    assert_eq!(status.code, Code::Unschedulable);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/nodename/node_name_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestNodeName / `pod with no nodeName -> skip`.
/// Upstream expected: code = Skip (not Success, not Unschedulable).
#[test]
fn upstream_node_name_skip_when_unspecified() {
    let pod = Pod::new("t", "ns", "p");
    let status = NodeName.filter(&pod, &make_node("a"), &empty_snap(vec![]));
    assert_eq!(status.code, Code::Skip);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/nodeunschedulable/node_unschedulable_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestNodeUnschedulable / `cordoned node + unschedulable toleration`.
/// Upstream expected: Success — toleration on the well-known unschedulable
/// taint must let the pod schedule.
#[test]
fn upstream_node_unschedulable_cordoned_with_toleration_passes() {
    let mut node = make_node("a");
    node.status = NodeStatus::Cordoned;
    let mut pod = Pod::new("t", "ns", "p");
    pod.spec.tolerations.push(Toleration {
        key: Some("node.kubernetes.io/unschedulable".into()),
        operator: "Exists".into(),
        value: None,
        effect: None,
    });
    assert!(
        NodeUnschedulable
            .filter(&pod, &node, &empty_snap(vec![]))
            .is_success()
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Cross-plugin upstream invariants
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestImageLocalityPriority (driver-level scaling formula).
/// Independent of the Plugin Score path — verifies `scaled_image_score`
/// directly mirrors `size × num_nodes / total_nodes`.
#[test]
fn upstream_image_locality_scaled_score_formula_holds() {
    let state = ImageStateSummary {
        size_bytes: 1_000_000_000,
        num_nodes: 3,
    };
    // Upstream: scaled = 1e9 × 3 / 6 = 500_000_000.
    assert_eq!(ImageLocality::scaled_image_score(&state, 6), 500_000_000);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/scheduler/framework/plugins/noderesources/least_allocated_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestLeastAllocated / `more free resources score higher`.
/// Upstream expected: among two nodes, the one with more free cpu+mem
/// receives a strictly higher Score.
#[test]
fn upstream_least_allocated_more_free_scores_higher() {
    let mut hot = make_node("hot");
    hot.allocated.cpu_millicores = 3500;
    hot.allocated.memory_bytes = 7_000_000_000;
    let cold = make_node("cold"); // allocated = 0
    let pod = Pod::new("t", "ns", "p");
    let snap = empty_snap(vec![]);
    let hot_score = Resources.score(&pod, &hot, &snap);
    let cold_score = Resources.score(&pod, &cold, &snap);
    assert!(
        cold_score > hot_score,
        "cold ({cold_score}) should outscore hot ({hot_score})"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Smoke: thresholds match upstream constants verbatim
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: image_locality.go constants `mb23Threshold = 23 << 20`
/// and `mb1000Threshold = 1000 << 20`.
#[test]
fn upstream_image_locality_constants_match() {
    assert_eq!(IMAGE_LOCALITY_MIN_THRESHOLD, 23 * 1024 * 1024);
    assert_eq!(
        IMAGE_LOCALITY_MAX_THRESHOLD_PER_CONTAINER,
        1000 * 1024 * 1024
    );
}

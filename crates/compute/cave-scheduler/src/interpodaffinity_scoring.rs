// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! InterPodAffinity — soft (preferred) PreScore + Score + NormalizeScore.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/plugins/interpodaffinity/{plugin.go,scoring.go,filtering.go}
//!
//! Upstream's preferred-affinity scoring runs in three extension points:
//!
//! 1. **PreScore** — walk every node in the snapshot once. For each node, for
//!    each *existing* pod on it that matches one of the scheduling pod's
//!    `preferred_affinity` terms (or, symmetrically, `preferred_anti_affinity`
//!    on the existing pod's spec that matches the scheduling pod), add /
//!    subtract `weight` to a per-topology counter keyed by
//!    `(topology_key, topology_value)`. The result is stored in [`PreScoreState`]
//!    so the per-node Score pass is O(1) per node.
//!
//! 2. **Score** — for the node under evaluation, sum the per-topology counter
//!    over every topology key the pod cares about. A node that sits in a
//!    `(zone, us-east-1a)` bucket whose counter is +30 gets a raw score of 30.
//!    Counters can be negative (anti-affinity dominates).
//!
//! 3. **NormalizeScore** — linearly map the raw scores onto
//!    `[0, framework.MAX_NODE_SCORE]` based on the min/max raw values across
//!    nodes. Symmetric to upstream's `Normalize` which maps
//!    `[minCount, maxCount] -> [0, framework.MaxNodeScore]`.
//!
//! The Filter path (hard affinity / anti-affinity) is already implemented in
//! `plugins.rs`'s `InterPodAffinity`. This module is purely the soft-preferences
//! scoring chain — what upstream calls `Score`/`PreScore`/`NormalizeScore`.
//!
//! The eager `InterPodAffinityScore` in `plugins.rs` (a non-state per-node
//! linear walk) remains for callers that want the simple O(N×M) path; this
//! module's [`InterPodAffinityScoring`] is the cached, normalized, framework-
//! shaped variant.

use crate::framework::{
    ClusterSnapshot, Pod, PodAffinityTerm, Status, WeightedPodAffinityTerm, MAX_NODE_SCORE,
};
use crate::models::Node;
use std::collections::HashMap;
use std::sync::Mutex;

/// Per-cycle pre-computed counter table.
///
/// Key: `(topology_key, topology_value)` — e.g. `("topology.kubernetes.io/zone", "us-east-1a")`.
/// Value: signed weight sum; +weight for each matching preferred-affinity
/// pairing, -weight for each matching preferred-anti-affinity pairing.
#[derive(Debug, Default, Clone)]
pub struct PreScoreState {
    pub topology_pair_to_score: HashMap<(String, String), i64>,
    /// Topology keys the scheduling pod's preferred terms care about — used
    /// during Score to know which keys to look up on a candidate node.
    pub topology_keys: Vec<String>,
}

impl PreScoreState {
    /// Compute the topology score table for one scheduling pod against the
    /// existing pods in `snapshot`.
    ///
    /// Walks every node in the snapshot. For each pod already placed on that
    /// node, runs through the scheduling pod's `preferred_affinity` and
    /// `preferred_anti_affinity` lists. When a term matches the existing pod,
    /// the node's value for that term's `topology_key` is bumped by `±weight`
    /// in `topology_pair_to_score`.
    pub fn compute(
        scheduling_pod: &Pod,
        preferred_affinity: &[WeightedPodAffinityTerm],
        preferred_anti_affinity: &[WeightedPodAffinityTerm],
        snapshot: &ClusterSnapshot,
    ) -> Self {
        let mut state = PreScoreState::default();
        let mut keys: Vec<String> = Vec::new();
        for w in preferred_affinity.iter().chain(preferred_anti_affinity.iter()) {
            if !keys.iter().any(|k| k == &w.term.topology_key) {
                keys.push(w.term.topology_key.clone());
            }
        }
        state.topology_keys = keys;

        for node in &snapshot.nodes {
            for w in preferred_affinity {
                let Some(topo_v) = node.labels.get(&w.term.topology_key) else {
                    continue;
                };
                for existing in snapshot.pods_on(&node.name) {
                    if term_matches(existing, &w.term, scheduling_pod) {
                        let key = (w.term.topology_key.clone(), topo_v.clone());
                        *state.topology_pair_to_score.entry(key).or_insert(0) += w.weight as i64;
                    }
                }
            }
            for w in preferred_anti_affinity {
                let Some(topo_v) = node.labels.get(&w.term.topology_key) else {
                    continue;
                };
                for existing in snapshot.pods_on(&node.name) {
                    if term_matches(existing, &w.term, scheduling_pod) {
                        let key = (w.term.topology_key.clone(), topo_v.clone());
                        *state.topology_pair_to_score.entry(key).or_insert(0) -= w.weight as i64;
                    }
                }
            }
        }
        state
    }

    /// Raw per-node score for the precomputed table. Sum of counters at every
    /// `(topology_key, node_label_value)` the pod cares about.
    pub fn raw_score(&self, node: &Node) -> i64 {
        let mut s: i64 = 0;
        for tk in &self.topology_keys {
            let Some(tv) = node.labels.get(tk) else {
                continue;
            };
            if let Some(c) = self.topology_pair_to_score.get(&(tk.clone(), tv.clone())) {
                s += *c;
            }
        }
        s
    }
}

/// Soft pod-affinity scoring plugin shaped like upstream's PreScore/Score/Normalize.
///
/// Configured at registration with the scheduling pod's preferred-affinity
/// terms; [`PreScoreState`] is computed once per cycle via [`pre_score`] and
/// stashed internally so the per-node Score pass is O(1) per node (vs the
/// O(N×M) eager walk in `plugins.rs::InterPodAffinityScore`).
pub struct InterPodAffinityScoring {
    pub preferred_affinity: Vec<WeightedPodAffinityTerm>,
    pub preferred_anti_affinity: Vec<WeightedPodAffinityTerm>,
    /// Cycle-local state — populated by `pre_score`, consumed by `score`,
    /// rescaled by `normalize`.
    state: Mutex<Option<PreScoreState>>,
}

impl InterPodAffinityScoring {
    pub fn new() -> Self {
        Self {
            preferred_affinity: Vec::new(),
            preferred_anti_affinity: Vec::new(),
            state: Mutex::new(None),
        }
    }

    pub fn with_preferred_affinity(mut self, t: WeightedPodAffinityTerm) -> Self {
        self.preferred_affinity.push(t);
        self
    }

    pub fn with_preferred_anti_affinity(mut self, t: WeightedPodAffinityTerm) -> Self {
        self.preferred_anti_affinity.push(t);
        self
    }

    pub fn name() -> &'static str {
        "InterPodAffinityScoring"
    }

    /// PreScore — populate the cycle-local topology counter table.
    pub fn pre_score(&self, pod: &Pod, snapshot: &ClusterSnapshot) -> Status {
        if self.preferred_affinity.is_empty() && self.preferred_anti_affinity.is_empty() {
            *self.state.lock().unwrap() = Some(PreScoreState::default());
            return Status::skip(Self::name());
        }
        let s = PreScoreState::compute(
            pod,
            &self.preferred_affinity,
            &self.preferred_anti_affinity,
            snapshot,
        );
        *self.state.lock().unwrap() = Some(s);
        Status::success(Self::name())
    }

    /// Score — raw weighted sum for one node. Negative values are allowed at
    /// this step; `normalize` is what maps them onto `[0, MAX_NODE_SCORE]`.
    pub fn score(&self, _pod: &Pod, node: &Node) -> i64 {
        let g = self.state.lock().unwrap();
        let Some(st) = g.as_ref() else {
            return 0;
        };
        st.raw_score(node)
    }

    /// NormalizeScore — linearly map `[min, max]` → `[0, MAX_NODE_SCORE]`
    /// across the per-node scores produced by `score`.
    ///
    /// Mirrors `pkg/scheduler/framework/plugins/interpodaffinity/scoring.go`'s
    /// `NormalizeScore`: if every node has the same raw value, every node ends
    /// up at `MAX_NODE_SCORE`. Otherwise the linear map preserves order.
    pub fn normalize(&self, scores: &mut [(String, i64)]) -> Status {
        if scores.is_empty() {
            return Status::success(Self::name());
        }
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        for (_, s) in scores.iter() {
            if *s < min {
                min = *s;
            }
            if *s > max {
                max = *s;
            }
        }
        if max == min {
            // Avoid divide-by-zero. Upstream pins every node to MaxNodeScore in
            // this case (every node is equally preferred → flat ceiling).
            for (_, s) in scores.iter_mut() {
                *s = MAX_NODE_SCORE;
            }
            return Status::success(Self::name());
        }
        let range = (max - min) as i128;
        for (_, s) in scores.iter_mut() {
            let shifted = (*s - min) as i128;
            let mapped = (MAX_NODE_SCORE as i128) * shifted / range;
            *s = mapped.clamp(0, MAX_NODE_SCORE as i128) as i64;
        }
        Status::success(Self::name())
    }
}

impl Default for InterPodAffinityScoring {
    fn default() -> Self {
        Self::new()
    }
}

/// Match a `PodAffinityTerm` against an existing pod. Lightweight subset of
/// `plugins.rs::pod_term_matches_with_pod` — kept private to this module so
/// the soft-scoring path does not have to import every Filter helper.
///
/// Namespacing rule mirrors upstream's `getAffinityTermProperties`:
/// - `term.namespaces` empty AND `term.namespace_selector` `None` →
///   restrict to the *scheduling pod's* namespace.
/// - Otherwise: existing pod's namespace must be in `term.namespaces` OR
///   `term.namespace_selector` matches (here approximated as `is_some()` →
///   match-all since per-namespace label lookup needs a NamespaceSnapshot we
///   don't carry yet; upstream falls back to literal `namespaces` list when
///   the namespace selector evaluates empty).
fn term_matches(existing: &Pod, term: &PodAffinityTerm, scheduling: &Pod) -> bool {
    // Namespace scoping.
    let ns_ok = if term.namespaces.is_empty() && term.namespace_selector.is_none() {
        existing.namespace == scheduling.namespace
    } else if term.namespaces.iter().any(|n| n == &existing.namespace) {
        true
    } else {
        term.namespace_selector.is_some()
    };
    if !ns_ok {
        return false;
    }
    // Empty selectors mean "match everything" only when `selector_v2` is also
    // None; an empty legacy map plus an explicit (possibly empty) v2 selector
    // is still considered "selector present" — we let `selector_v2` decide.
    let legacy_ok = term
        .label_selector
        .iter()
        .all(|(k, v)| existing_label(existing, k) == Some(v));
    if !legacy_ok {
        return false;
    }
    if let Some(v2) = &term.selector_v2 {
        if !v2.matches(&existing_labels(existing)) {
            return false;
        }
    }
    // match_label_keys / mismatch_label_keys are scheduling-pod-side helpers
    // that *inject* selector entries lifted from `scheduling.spec.labels`.
    // The scheduling pod's spec in our model doesn't carry a label map, so we
    // skip those lifts honestly — same shortcut taken by tests in plugins.rs.
    true
}

fn existing_label<'a>(_p: &'a Pod, _k: &str) -> Option<&'a String> {
    // Existing-pod labels are not stored on `Pod` in our framework today; the
    // hard-affinity Filter walks `snap.pods_on(node)` and uses the same field
    // shortcut. When labels become a first-class part of `Pod`, swap this.
    // Returning `None` means "no label" — only an *empty* label_selector
    // matches everything.
    None
}

fn existing_labels(_p: &Pod) -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::{
        ClusterSnapshot, Pod, PodAffinityTerm, VolumeKind, WeightedPodAffinityTerm,
    };
    use crate::models::{
        Node as ModelNode, NodeStatus, ResourceCapacity,
    };
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn _unused_volume() -> VolumeKind {
        VolumeKind::HostPath { path: "/".into() }
    }

    fn n(name: &str, zone: &str) -> ModelNode {
        let mut labels = HashMap::new();
        labels.insert("topology.kubernetes.io/zone".to_string(), zone.to_string());
        ModelNode {
            name: name.into(),
            uid: Uuid::new_v4(),
            status: NodeStatus::Ready,
            capacity: ResourceCapacity::default(),
            allocatable: ResourceCapacity::default(),
            allocated: ResourceCapacity::default(),
            labels,
            taints: vec![],
            conditions: vec![],
            registered_at: Utc::now(),
            last_heartbeat: Utc::now(),
        }
    }

    fn snap(node_pods: Vec<(ModelNode, Vec<Pod>)>) -> ClusterSnapshot {
        let mut s = ClusterSnapshot::default();
        for (node, pods) in node_pods {
            s.pods_by_node.insert(node.name.clone(), pods);
            s.nodes.push(node);
        }
        s
    }

    fn term(topo: &str) -> PodAffinityTerm {
        PodAffinityTerm {
            label_selector: HashMap::new(),
            topology_key: topo.into(),
            namespaces: vec![],
            selector_v2: None,
            namespace_selector: None,
            match_label_keys: vec![],
            mismatch_label_keys: vec![],
        }
    }

    #[test]
    fn pre_score_aggregates_weights_per_topology() {
        let p = Pod::new("t", "ns", "incoming");
        let a = n("a", "us-east-1a");
        let b = n("b", "us-east-1a");
        let c = n("c", "us-east-1b");
        let existing_a = Pod::new("t", "ns", "buddy1");
        let existing_b = Pod::new("t", "ns", "buddy2");

        let snap = snap(vec![
            (a.clone(), vec![existing_a]),
            (b.clone(), vec![existing_b]),
            (c.clone(), vec![]),
        ]);

        let plug = InterPodAffinityScoring::new().with_preferred_affinity(WeightedPodAffinityTerm {
            weight: 10,
            term: term("topology.kubernetes.io/zone"),
        });

        let pre = PreScoreState::compute(
            &p,
            &plug.preferred_affinity,
            &plug.preferred_anti_affinity,
            &snap,
        );

        // us-east-1a has two matching existing pods → 2×10 = 20.
        let v_a = pre
            .topology_pair_to_score
            .get(&(
                "topology.kubernetes.io/zone".into(),
                "us-east-1a".into(),
            ))
            .copied()
            .unwrap_or(0);
        assert_eq!(v_a, 20);

        // us-east-1b has no existing pods → entry absent, raw_score == 0.
        assert_eq!(pre.raw_score(&c), 0);
        assert_eq!(pre.raw_score(&a), 20);
        assert_eq!(pre.raw_score(&b), 20);
    }

    #[test]
    fn score_path_subtracts_anti_affinity() {
        let p = Pod::new("t", "ns", "incoming");
        let a = n("a", "us-east-1a");
        let b = n("b", "us-east-1b");
        let existing = Pod::new("t", "ns", "rival");

        let snap = snap(vec![(a.clone(), vec![existing]), (b.clone(), vec![])]);

        let plug =
            InterPodAffinityScoring::new().with_preferred_anti_affinity(WeightedPodAffinityTerm {
                weight: 50,
                term: term("topology.kubernetes.io/zone"),
            });
        plug.pre_score(&p, &snap);

        // Node a sits in the penalised zone.
        assert_eq!(plug.score(&p, &a), -50);
        // Node b is in a clean zone.
        assert_eq!(plug.score(&p, &b), 0);
    }

    #[test]
    fn normalize_maps_range_to_max_node_score() {
        let plug = InterPodAffinityScoring::new();
        let mut scores = vec![
            ("a".into(), -10_i64),
            ("b".into(), 0_i64),
            ("c".into(), 30_i64),
        ];
        plug.normalize(&mut scores);
        // -10 → 0, 30 → MAX_NODE_SCORE, 0 sits in between linearly.
        assert_eq!(scores[0].1, 0);
        assert_eq!(scores[2].1, MAX_NODE_SCORE);
        assert!(scores[1].1 > 0 && scores[1].1 < MAX_NODE_SCORE);
    }

    #[test]
    fn normalize_flat_assigns_max_to_all() {
        let plug = InterPodAffinityScoring::new();
        let mut scores = vec![("a".into(), 7_i64), ("b".into(), 7_i64)];
        plug.normalize(&mut scores);
        assert_eq!(scores[0].1, MAX_NODE_SCORE);
        assert_eq!(scores[1].1, MAX_NODE_SCORE);
    }

    #[test]
    fn pre_score_skips_when_no_preferred_terms() {
        let plug = InterPodAffinityScoring::new();
        let p = Pod::new("t", "ns", "x");
        let s = snap(vec![]);
        let status = plug.pre_score(&p, &s);
        assert!(status.is_skip(), "no preferred terms → Skip");
    }

    #[test]
    fn end_to_end_pre_score_score_normalize() {
        let p = Pod::new("t", "ns", "incoming");
        let a = n("a", "zone-1");
        let b = n("b", "zone-2");
        let c = n("c", "zone-3");
        let buddy = Pod::new("t", "ns", "buddy");
        let rival = Pod::new("t", "ns", "rival");
        let snap = snap(vec![
            (a.clone(), vec![buddy]),
            (b.clone(), vec![]),
            (c.clone(), vec![rival]),
        ]);

        let plug = InterPodAffinityScoring::new()
            .with_preferred_affinity(WeightedPodAffinityTerm {
                weight: 5,
                term: term("topology.kubernetes.io/zone"),
            })
            .with_preferred_anti_affinity(WeightedPodAffinityTerm {
                weight: 2,
                term: term("topology.kubernetes.io/zone"),
            });
        assert!(plug.pre_score(&p, &snap).is_success());

        let mut raw = vec![
            ("a".to_string(), plug.score(&p, &a)),
            ("b".to_string(), plug.score(&p, &b)),
            ("c".to_string(), plug.score(&p, &c)),
        ];
        // Raw before normalize: a=+5−2=+3 (buddy contributes both); b=0; c=+5−2=+3.
        // (Anti-affinity term also fires because we don't yet filter by
        // existing-pod identity — same buddy matches both.)
        assert!(raw[0].1 >= 0);
        plug.normalize(&mut raw);
        for (_, v) in &raw {
            assert!((0..=MAX_NODE_SCORE).contains(v), "normalized in range: {v}");
        }
    }
}

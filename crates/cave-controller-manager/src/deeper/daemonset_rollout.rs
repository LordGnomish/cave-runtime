// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DaemonSet — rolling update partition + taint/toleration evaluator.
//!
//! Mirrors `pkg/controller/daemon/update.go::rollingUpdate` plus the shared
//! `pkg/util/tolerations/tolerations.go::TolerationsTolerateTaint` helper.

use crate::types::{Cite, ControllerError, TenantId};
use serde::{Deserialize, Serialize};

/// Mirrors `core/v1.TaintEffect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: Option<String>,
    pub effect: TaintEffect,
}

/// Mirrors `core/v1.TolerationOperator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TolerationOp {
    Equal,
    Exists,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toleration {
    pub key: Option<String>,
    pub operator: TolerationOp,
    pub value: Option<String>,
    /// `None` matches every effect.
    pub effect: Option<TaintEffect>,
}

/// True if `tol` tolerates `taint`. Mirrors `Toleration.ToleratesTaint`.
pub fn toleration_tolerates_taint(tol: &Toleration, taint: &Taint) -> bool {
    if let Some(e) = tol.effect {
        if e != taint.effect {
            return false;
        }
    }
    match tol.operator {
        TolerationOp::Exists => match &tol.key {
            None => true, // matches every taint
            Some(k) => k == &taint.key,
        },
        TolerationOp::Equal => match (&tol.key, &tol.value) {
            (Some(k), Some(v)) => k == &taint.key && Some(v) == taint.value.as_ref(),
            _ => false, // Equal requires both
        },
    }
}

/// True iff every taint with effect ∈ {`NoSchedule`, `NoExecute`} is
/// tolerated. Mirrors `TolerationsTolerateTaintsWithFilter` upstream
/// (filter = effect != PreferNoSchedule).
pub fn tolerations_tolerate_taints(tols: &[Toleration], taints: &[Taint]) -> bool {
    for t in taints {
        if t.effect == TaintEffect::PreferNoSchedule {
            continue;
        }
        if !tols.iter().any(|tol| toleration_tolerates_taint(tol, t)) {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeView {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub taints: Vec<Taint>,
    /// `Some(generation)` if the DS pod on this node already runs the
    /// controller's current revision; `None` if it hasn't been rolled yet.
    pub current_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonSetSpec {
    pub name: String,
    pub tenant: TenantId,
    /// Required label match — empty selector matches every node.
    pub node_selector: Vec<(String, String)>,
    pub tolerations: Vec<Toleration>,
    pub revision: u64,
    /// Maximum nodes that may be running the *old* revision concurrently.
    /// Mirrors `RollingUpdate.MaxUnavailable`.
    pub max_unavailable: u32,
}

/// Returns true iff the node's labels satisfy the spec's `node_selector`.
pub fn node_matches_selector(spec: &DaemonSetSpec, node: &NodeView) -> bool {
    spec.node_selector
        .iter()
        .all(|(k, v)| node.labels.iter().any(|(nk, nv)| nk == k && nv == v))
}

/// Should the DS run on this node? Combines selector + taint tolerance.
pub fn node_should_run(spec: &DaemonSetSpec, node: &NodeView) -> bool {
    if !node_matches_selector(spec, node) {
        return false;
    }
    tolerations_tolerate_taints(&spec.tolerations, &node.taints)
}

/// One rolling-update decision: which nodes to roll right now, given the
/// `max_unavailable` budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollPlan {
    /// Nodes the DS should run on (selector + taints).
    pub eligible_nodes: Vec<String>,
    /// Subset whose pod is on the old revision.
    pub stale_nodes: Vec<String>,
    /// Subset that should be rolled in this pass.
    pub roll_now: Vec<String>,
    /// Spare slack — how many more nodes could be unavailable simultaneously.
    pub remaining_budget: u32,
}

/// Compute a rolling-update plan. Mirrors `rollingUpdate` upstream.
///
/// `currently_unavailable` is the number of nodes whose DS pod is *already*
/// being recreated; it counts against the `max_unavailable` budget.
pub fn plan_rollout(
    spec: &DaemonSetSpec,
    nodes: &[NodeView],
    caller: &TenantId,
    currently_unavailable: u32,
) -> Result<RollPlan, ControllerError> {
    if caller != &spec.tenant {
        return Err(ControllerError::TenantDenied {
            tenant: caller.clone(),
            kind: "DaemonSet",
            name: spec.name.clone(),
        });
    }
    let eligible: Vec<String> = nodes
        .iter()
        .filter(|n| node_should_run(spec, n))
        .map(|n| n.name.clone())
        .collect();
    let stale: Vec<String> = nodes
        .iter()
        .filter(|n| node_should_run(spec, n) && n.current_revision != Some(spec.revision))
        .map(|n| n.name.clone())
        .collect();
    let budget = spec.max_unavailable.saturating_sub(currently_unavailable);
    let roll_now: Vec<String> = stale.iter().take(budget as usize).cloned().collect();
    let remaining_budget = budget.saturating_sub(roll_now.len() as u32);
    Ok(RollPlan { eligible_nodes: eligible, stale_nodes: stale, roll_now, remaining_budget })
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/daemon/update.go", "rollingUpdate");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn taint(k: &str, v: Option<&str>, e: TaintEffect) -> Taint {
        Taint { key: k.into(), value: v.map(|s| s.to_string()), effect: e }
    }
    fn tol_eq(k: &str, v: &str, e: Option<TaintEffect>) -> Toleration {
        Toleration { key: Some(k.into()), operator: TolerationOp::Equal, value: Some(v.into()), effect: e }
    }
    fn tol_exists(k: Option<&str>, e: Option<TaintEffect>) -> Toleration {
        Toleration { key: k.map(|s| s.to_string()), operator: TolerationOp::Exists, value: None, effect: e }
    }
    fn node(name: &str, labels: &[(&str, &str)], taints: Vec<Taint>, rev: Option<u64>) -> NodeView {
        NodeView {
            name: name.into(),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            taints,
            current_revision: rev,
        }
    }
    fn ds(rev: u64, max_unavail: u32, sel: &[(&str, &str)], tols: Vec<Toleration>) -> DaemonSetSpec {
        DaemonSetSpec {
            name: "node-exporter".into(),
            tenant: TenantId::new("acme").expect("test fixture"),
            node_selector: sel.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            tolerations: tols,
            revision: rev,
            max_unavailable: max_unavail,
        }
    }

    #[test]
    fn equal_toleration_with_matching_kv_tolerates_taint() {
        let (_cite, _t) = test_ctx!(
            "pkg/util/tolerations/tolerations.go",
            "Toleration.ToleratesTaint",
            "tenant-ds-tol-eq"
        );
        let t = taint("dedicated", Some("gpu"), TaintEffect::NoSchedule);
        let tol = tol_eq("dedicated", "gpu", Some(TaintEffect::NoSchedule));
        assert!(toleration_tolerates_taint(&tol, &t));
    }

    #[test]
    fn equal_toleration_with_mismatched_value_does_not_tolerate() {
        let (_cite, _t) = test_ctx!(
            "pkg/util/tolerations/tolerations.go",
            "Toleration.ToleratesTaint",
            "tenant-ds-tol-mismatch"
        );
        let t = taint("dedicated", Some("gpu"), TaintEffect::NoSchedule);
        let tol = tol_eq("dedicated", "tpu", Some(TaintEffect::NoSchedule));
        assert!(!toleration_tolerates_taint(&tol, &t));
    }

    #[test]
    fn exists_with_no_key_tolerates_every_taint() {
        let (_cite, _t) = test_ctx!(
            "pkg/util/tolerations/tolerations.go",
            "Toleration.ToleratesTaint",
            "tenant-ds-tol-exists-any"
        );
        let universal = tol_exists(None, None);
        for e in [TaintEffect::NoSchedule, TaintEffect::PreferNoSchedule, TaintEffect::NoExecute] {
            assert!(toleration_tolerates_taint(&universal, &taint("anything", Some("x"), e)));
        }
    }

    #[test]
    fn prefer_no_schedule_taints_are_ignored_by_aggregator() {
        let (_cite, _t) = test_ctx!(
            "pkg/util/tolerations/tolerations.go",
            "TolerationsTolerateTaintsWithFilter",
            "tenant-ds-prefer"
        );
        let taints = vec![taint("nice-to-have", None, TaintEffect::PreferNoSchedule)];
        // No tolerations at all, but PreferNoSchedule is filtered out.
        assert!(tolerations_tolerate_taints(&[], &taints));
    }

    #[test]
    fn no_execute_without_toleration_blocks_node() {
        let (_cite, _t) = test_ctx!(
            "pkg/util/tolerations/tolerations.go",
            "TolerationsTolerateTaintsWithFilter",
            "tenant-ds-noexec"
        );
        let taints = vec![taint("evict", None, TaintEffect::NoExecute)];
        assert!(!tolerations_tolerate_taints(&[], &taints));
    }

    #[test]
    fn node_should_run_combines_selector_and_taints() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "nodeShouldRunDaemonPod",
            "tenant-ds-should-run"
        );
        let s = ds(1, 1, &[("role", "edge")], vec![tol_exists(Some("dedicated"), Some(TaintEffect::NoSchedule))]);
        let n_match = node(
            "edge-1",
            &[("role", "edge")],
            vec![taint("dedicated", Some("ml"), TaintEffect::NoSchedule)],
            Some(1),
        );
        let n_no_label = node("core-1", &[("role", "core")], vec![], Some(1));
        let n_no_tol = node(
            "edge-2",
            &[("role", "edge")],
            vec![taint("strict", None, TaintEffect::NoSchedule)],
            Some(1),
        );
        assert!(node_should_run(&s, &n_match));
        assert!(!node_should_run(&s, &n_no_label));
        assert!(!node_should_run(&s, &n_no_tol));
    }

    #[test]
    fn rollout_plan_caps_concurrent_rolls_at_max_unavailable() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "acme"
        );
        let s = ds(2, 2, &[], vec![]);
        let nodes: Vec<NodeView> = (0..5)
            .map(|i| node(&format!("n{i}"), &[], vec![], Some(1))) // all stale
            .collect();
        let plan = plan_rollout(&s, &nodes, &tenant, 0).unwrap();
        assert_eq!(plan.stale_nodes.len(), 5);
        assert_eq!(plan.roll_now.len(), 2);
        assert_eq!(plan.remaining_budget, 0);
    }

    #[test]
    fn rollout_plan_subtracts_currently_unavailable_from_budget() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "acme"
        );
        let s = ds(2, 3, &[], vec![]);
        let nodes: Vec<NodeView> = (0..5)
            .map(|i| node(&format!("n{i}"), &[], vec![], Some(1)))
            .collect();
        let plan = plan_rollout(&s, &nodes, &tenant, 2).unwrap();
        assert_eq!(plan.roll_now.len(), 1); // 3 - 2 = 1
    }

    #[test]
    fn rollout_plan_skips_nodes_outside_selector_or_intolerable() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "podsShouldBeOnNode",
            "acme"
        );
        let s = ds(2, 5, &[("role", "edge")], vec![]);
        let nodes = vec![
            node("edge-1", &[("role", "edge")], vec![], Some(1)),    // stale
            node("edge-2", &[("role", "edge")], vec![], Some(2)),    // current
            node("core-1", &[("role", "core")], vec![], Some(1)),    // wrong selector
            node(
                "edge-tainted",
                &[("role", "edge")],
                vec![taint("locked", None, TaintEffect::NoSchedule)],
                Some(1),
            ),
        ];
        let plan = plan_rollout(&s, &nodes, &tenant, 0).unwrap();
        assert_eq!(plan.eligible_nodes, vec!["edge-1".to_string(), "edge-2".to_string()]);
        assert_eq!(plan.stale_nodes, vec!["edge-1".to_string()]);
    }

    #[test]
    fn rollout_plan_refuses_cross_tenant_caller() {
        let (_cite, attacker) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "tenant-attacker"
        );
        let s = ds(1, 1, &[], vec![]);
        let err = plan_rollout(&s, &[], &attacker, 0).unwrap_err();
        assert!(matches!(err, ControllerError::TenantDenied { .. }));
    }
}

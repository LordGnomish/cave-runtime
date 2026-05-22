// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DaemonSet controller — one pod per (eligible) node.
//!
//! Upstream: [`pkg/controller/daemon`]. The full controller computes the
//! schedulability of each node, respects taints/tolerations, and runs a
//! rolling update similar to Deployment.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSetSpec {
    pub name: String,
    pub namespace: String,
    /// Optional node selector. Empty = match every node.
    pub node_selector: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeView {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub schedulable: bool,
    pub running_ds_pod: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonSetStatus {
    pub desired_number_scheduled: u32,
    pub current_number_scheduled: u32,
    pub number_ready: u32,
}

/// Returns true if `node` matches the DaemonSet's selector and is schedulable.
/// Mirrors `nodeShouldRunDaemonPod` in `pkg/controller/daemon/daemon_controller.go`.
pub fn node_should_run(spec: &DaemonSetSpec, node: &NodeView) -> bool {
    if !node.schedulable {
        return false;
    }
    spec.node_selector
        .iter()
        .all(|(k, v)| node.labels.iter().any(|(nk, nv)| nk == k && nv == v))
}

/// Mirrors `manage` in `pkg/controller/daemon/daemon_controller.go`.
pub fn reconcile(
    spec: &DaemonSetSpec,
    nodes: &[NodeView],
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    let mut creates: u32 = 0;
    let mut deletes: u32 = 0;
    for n in nodes {
        let want = node_should_run(spec, n);
        match (want, n.running_ds_pod) {
            (true, false) => creates += 1,
            (false, true) => deletes += 1,
            _ => {}
        }
    }
    if creates == 0 && deletes == 0 {
        Ok(Reconcile::NoOp)
    } else if creates >= deletes {
        Ok(Reconcile::Create(creates))
    } else {
        Ok(Reconcile::Delete(deletes))
    }
}

/// Taint effect — mirrors `core/v1.TaintEffect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: Option<String>,
    pub effect: TaintEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TolerationOperator {
    Equal,
    Exists,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Toleration {
    pub key: Option<String>,
    pub operator: TolerationOperator,
    pub value: Option<String>,
    pub effect: Option<TaintEffect>,
}

/// Whether a single toleration covers a single taint. Mirrors
/// `apimachinery/pkg/api/v1/helper/helpers.go::Toleration::ToleratesTaint`.
pub fn tolerates(toleration: &Toleration, taint: &Taint) -> bool {
    if let Some(eff) = toleration.effect {
        if eff != taint.effect {
            return false;
        }
    }
    match toleration.operator {
        TolerationOperator::Exists => {
            // empty key matches all keys
            toleration
                .key
                .as_deref()
                .map(|k| k == taint.key)
                .unwrap_or(true)
        }
        TolerationOperator::Equal => {
            toleration.key.as_deref() == Some(taint.key.as_str())
                && toleration.value.as_deref() == taint.value.as_deref()
        }
    }
}

/// Whether a set of tolerations covers every NoSchedule/NoExecute taint
/// on the node. Mirrors
/// `apimachinery/pkg/api/v1/helper/helpers.go::FindMatchingUntoleratedTaint`.
pub fn tolerates_all(taints: &[Taint], tolerations: &[Toleration]) -> bool {
    taints
        .iter()
        .filter(|t| matches!(t.effect, TaintEffect::NoSchedule | TaintEffect::NoExecute))
        .all(|t| tolerations.iter().any(|tol| tolerates(tol, t)))
}

/// Plan one rolling-update step for a DaemonSet — pick at most
/// `max_unavailable` nodes whose pod is on an outdated revision and
/// schedule a replacement. Mirrors `pkg/controller/daemon/update.go::rollingUpdate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonNodeView {
    pub name: String,
    pub at_current_revision: bool,
    pub running_ds_pod: bool,
}

pub fn rolling_update(
    nodes: &[DaemonNodeView],
    max_unavailable: u32,
) -> Result<Reconcile, ControllerError> {
    let outdated: Vec<&DaemonNodeView> = nodes
        .iter()
        .filter(|n| n.running_ds_pod && !n.at_current_revision)
        .collect();
    if outdated.is_empty() {
        return Ok(Reconcile::NoOp);
    }
    let to_update = (outdated.len() as u32).min(max_unavailable.max(1));
    Ok(Reconcile::Update(to_update))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/daemon/daemon_controller.go",
    "DaemonSetsController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ds(selector: Vec<(&str, &str)>) -> DaemonSetSpec {
        DaemonSetSpec {
            name: "node-exporter".into(),
            namespace: "monitoring".into(),
            node_selector: selector
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn node(name: &str, labels: &[(&str, &str)], schedulable: bool, has_pod: bool) -> NodeView {
        NodeView {
            name: name.into(),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            schedulable,
            running_ds_pod: has_pod,
        }
    }

    #[test]
    fn matches_every_schedulable_node_with_no_selector() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "nodeShouldRunDaemonPod",
            "tenant-ds-no-selector"
        );
        let s = ds(vec![]);
        let nodes = vec![node("a", &[], true, false), node("b", &[], false, false)];
        assert!(node_should_run(&s, &nodes[0]));
        assert!(!node_should_run(&s, &nodes[1]));
        assert_eq!(
            reconcile(&s, &nodes, &tenant).unwrap(),
            Reconcile::Create(1)
        );
    }

    #[test]
    fn selector_filters_out_unlabeled_nodes() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "nodeShouldRunDaemonPod",
            "tenant-ds-selector"
        );
        let s = ds(vec![("role", "edge")]);
        let nodes = vec![
            node("edge-1", &[("role", "edge")], true, false),
            node("core-1", &[("role", "core")], true, false),
        ];
        assert_eq!(
            reconcile(&s, &nodes, &tenant).unwrap(),
            Reconcile::Create(1)
        );
    }

    #[test]
    fn deletes_pod_when_node_no_longer_eligible() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "manage",
            "tenant-ds-evict"
        );
        let s = ds(vec![("role", "edge")]);
        let nodes = vec![node("former-edge", &[("role", "core")], true, true)];
        assert_eq!(
            reconcile(&s, &nodes, &tenant).unwrap(),
            Reconcile::Delete(1)
        );
    }

    #[test]
    fn no_op_when_every_node_already_correct() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "manage",
            "tenant-ds-noop"
        );
        let s = ds(vec![]);
        let nodes = vec![node("a", &[], true, true), node("b", &[], true, true)];
        assert_eq!(reconcile(&s, &nodes, &tenant).unwrap(), Reconcile::NoOp);
    }

    // ── Deeper coverage (deeper-001) ─────────────────────────────────────────

    fn taint(key: &str, value: Option<&str>, effect: TaintEffect) -> Taint {
        Taint {
            key: key.into(),
            value: value.map(String::from),
            effect,
        }
    }

    /// Upstream parity: `TestToleration_ExistsMatchesAnyValue`
    /// (apimachinery/pkg/api/v1/helper/helpers_test.go::TestToleratesTaint —
    /// `Operator: Exists` matches any value of the same key).
    #[test]
    fn toleration_exists_operator_matches_any_value_for_key() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/apimachinery/pkg/api/v1/helper/helpers.go",
            "ToleratesTaint",
            "tenant-ds-tol-exists"
        );
        let _ = tenant;
        let tol = Toleration {
            key: Some("node-role.kubernetes.io/control-plane".into()),
            operator: TolerationOperator::Exists,
            value: None,
            effect: Some(TaintEffect::NoSchedule),
        };
        let t = taint(
            "node-role.kubernetes.io/control-plane",
            None,
            TaintEffect::NoSchedule,
        );
        assert!(tolerates(&tol, &t));
    }

    /// Upstream parity: `TestToleration_EqualOperatorRequiresValueMatch`
    /// (helpers_test.go::TestToleratesTaint — `Operator: Equal` requires
    /// both key and value to match exactly).
    #[test]
    fn toleration_equal_operator_requires_exact_value_match() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/apimachinery/pkg/api/v1/helper/helpers.go",
            "ToleratesTaint",
            "tenant-ds-tol-equal"
        );
        let _ = tenant;
        let tol = Toleration {
            key: Some("dedicated".into()),
            operator: TolerationOperator::Equal,
            value: Some("gpu".into()),
            effect: Some(TaintEffect::NoSchedule),
        };
        let hit = taint("dedicated", Some("gpu"), TaintEffect::NoSchedule);
        let miss = taint("dedicated", Some("cpu"), TaintEffect::NoSchedule);
        assert!(tolerates(&tol, &hit));
        assert!(!tolerates(&tol, &miss));
    }

    /// Upstream parity: `TestTolerations_UntoleratedTaintBlocksScheduling`
    /// (helpers_test.go::TestFindMatchingUntoleratedTaint — a single
    /// untolerated NoSchedule taint blocks the whole match).
    #[test]
    fn untolerated_no_schedule_taint_fails_match_all() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/apimachinery/pkg/api/v1/helper/helpers.go",
            "FindMatchingUntoleratedTaint",
            "tenant-ds-tol-untolerated"
        );
        let _ = tenant;
        let taints = vec![
            taint("dedicated", Some("gpu"), TaintEffect::NoSchedule),
            taint("disk", Some("ssd"), TaintEffect::NoSchedule),
        ];
        let tols = vec![Toleration {
            key: Some("dedicated".into()),
            operator: TolerationOperator::Equal,
            value: Some("gpu".into()),
            effect: Some(TaintEffect::NoSchedule),
        }];
        assert!(
            !tolerates_all(&taints, &tols),
            "missing toleration for `disk` blocks the match"
        );
    }

    /// Upstream parity: `TestTolerations_PreferNoScheduleIsAdvisory`
    /// (helpers_test.go — PreferNoSchedule is not enforced; even untolerated
    /// taints with that effect must NOT block scheduling).
    #[test]
    fn prefer_no_schedule_taints_do_not_block_match() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/apimachinery/pkg/api/v1/helper/helpers.go",
            "FindMatchingUntoleratedTaint",
            "tenant-ds-tol-prefer"
        );
        let _ = tenant;
        let taints = vec![taint("nice-to-have", None, TaintEffect::PreferNoSchedule)];
        // No tolerations at all.
        let tols: Vec<Toleration> = vec![];
        assert!(
            tolerates_all(&taints, &tols),
            "PreferNoSchedule is advisory only, never a hard match failure"
        );
    }

    /// Upstream parity: `TestDaemonRollingUpdate_RespectsMaxUnavailable`
    /// (pkg/controller/daemon/update_test.go::TestRollingUpdateDeployment —
    /// no more than `max_unavailable` outdated pods are updated per pass).
    #[test]
    fn rolling_update_caps_concurrent_updates_at_max_unavailable() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "tenant-ds-rolling-cap"
        );
        let _ = tenant;
        let nodes = vec![
            DaemonNodeView {
                name: "a".into(),
                at_current_revision: false,
                running_ds_pod: true,
            },
            DaemonNodeView {
                name: "b".into(),
                at_current_revision: false,
                running_ds_pod: true,
            },
            DaemonNodeView {
                name: "c".into(),
                at_current_revision: false,
                running_ds_pod: true,
            },
        ];
        let dec = rolling_update(&nodes, 2).unwrap();
        assert_eq!(
            dec,
            Reconcile::Update(2),
            "at most max_unavailable=2 outdated pods updated per pass"
        );
    }

    /// Upstream parity: `TestDaemonRollingUpdate_NoOpAtCurrentRevision`
    /// (update_test.go — NoOp once every running pod is at the current
    /// template revision).
    #[test]
    fn rolling_update_is_noop_when_every_pod_is_current() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "tenant-ds-rolling-noop"
        );
        let _ = tenant;
        let nodes = vec![
            DaemonNodeView {
                name: "a".into(),
                at_current_revision: true,
                running_ds_pod: true,
            },
            DaemonNodeView {
                name: "b".into(),
                at_current_revision: true,
                running_ds_pod: true,
            },
        ];
        let dec = rolling_update(&nodes, 2).unwrap();
        assert_eq!(dec, Reconcile::NoOp);
    }
}

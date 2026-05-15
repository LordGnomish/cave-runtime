// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DaemonSet update strategies — `pkg/controller/daemon/update.go`.
//!
//! `UpdateStrategy.type` is one of:
//!
//! * `RollingUpdate` — the controller deletes pods on nodes whose pod is
//!   not at the desired hash, gated by `maxUnavailable` and (since v1.21)
//!   `maxSurge`.
//! * `OnDelete` — the controller does NOT touch existing pods; users
//!   manually delete pods to trigger the new template.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateStrategyKind {
    RollingUpdate,
    OnDelete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStrategy {
    pub kind: UpdateStrategyKind,
    /// Only meaningful for RollingUpdate.
    pub max_unavailable: u32,
    /// Only meaningful for RollingUpdate; v1.21+.
    pub max_surge: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    pub node: String,
    /// Hash of the pod's template — `None` means the node has no DS pod yet.
    pub current_pod_hash: Option<String>,
    pub pod_ready: bool,
}

/// Compute the per-node action under the strategy. Mirrors
/// `pkg/controller/daemon/update.go::rollingUpdate` and the OnDelete branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeAction {
    /// Pod already at the desired hash — leave it.
    Keep,
    /// Pod is at an older hash — delete to trigger reschedule (rolling).
    DeletePod,
    /// Node has no pod yet — create one.
    CreatePod,
    /// OnDelete strategy: never delete, only create when missing.
    AwaitManualDelete,
}

pub fn plan_node_actions(
    strategy: &UpdateStrategy,
    desired_hash: &str,
    nodes: &[NodeView],
) -> Result<Vec<(String, NodeAction)>, ControllerError> {
    if strategy.max_unavailable == 0 && strategy.kind == UpdateStrategyKind::RollingUpdate {
        // Upstream rejects max_unavailable=0% AND max_surge=0; we model only
        // max_unavailable.
        if strategy.max_surge == 0 {
            return Err(ControllerError::InvalidSpec {
                kind: "DaemonSet",
                reason: "RollingUpdate requires maxUnavailable > 0 or maxSurge > 0".into(),
            });
        }
    }
    let mut budget = strategy.max_unavailable;
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        let action = match (&n.current_pod_hash, strategy.kind) {
            (None, _) if strategy.kind == UpdateStrategyKind::OnDelete => {
                NodeAction::CreatePod
            }
            (None, _) => NodeAction::CreatePod,
            (Some(h), UpdateStrategyKind::RollingUpdate) if h == desired_hash => NodeAction::Keep,
            (Some(_), UpdateStrategyKind::RollingUpdate) => {
                if budget > 0 {
                    budget -= 1;
                    NodeAction::DeletePod
                } else {
                    NodeAction::Keep
                }
            }
            (Some(h), UpdateStrategyKind::OnDelete) if h == desired_hash => NodeAction::Keep,
            (Some(_), UpdateStrategyKind::OnDelete) => NodeAction::AwaitManualDelete,
        };
        out.push((n.node.clone(), action));
    }
    Ok(out)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/daemon/update.go",
    "rollingUpdate",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn nv(node: &str, hash: Option<&str>, ready: bool) -> NodeView {
        NodeView {
            node: node.into(),
            current_pod_hash: hash.map(|s| s.to_string()),
            pod_ready: ready,
        }
    }
    fn rolling(max_unavailable: u32) -> UpdateStrategy {
        UpdateStrategy {
            kind: UpdateStrategyKind::RollingUpdate,
            max_unavailable,
            max_surge: 0,
        }
    }
    fn on_delete() -> UpdateStrategy {
        UpdateStrategy {
            kind: UpdateStrategyKind::OnDelete,
            max_unavailable: 0,
            max_surge: 0,
        }
    }

    #[test]
    fn rolling_keeps_node_already_at_desired() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "tenant-ds-roll-keep"
        );
        let plan = plan_node_actions(&rolling(2), "h1", &[nv("n1", Some("h1"), true)]).unwrap();
        assert_eq!(plan[0].1, NodeAction::Keep);
    }

    #[test]
    fn rolling_deletes_old_within_budget() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "tenant-ds-roll-delete"
        );
        let nodes = vec![
            nv("n1", Some("old"), true),
            nv("n2", Some("old"), true),
            nv("n3", Some("old"), true),
        ];
        let plan = plan_node_actions(&rolling(2), "new", &nodes).unwrap();
        let deletes = plan.iter().filter(|(_, a)| *a == NodeAction::DeletePod).count();
        assert_eq!(deletes, 2);
    }

    #[test]
    fn rolling_creates_when_node_missing_pod() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "manage",
            "tenant-ds-roll-create"
        );
        let plan = plan_node_actions(&rolling(2), "new", &[nv("n1", None, false)]).unwrap();
        assert_eq!(plan[0].1, NodeAction::CreatePod);
    }

    #[test]
    fn rolling_zero_budget_keeps_old_pods() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "rollingUpdate",
            "tenant-ds-roll-zero-budget"
        );
        let nodes = vec![nv("n1", Some("old"), true)];
        let s = UpdateStrategy {
            kind: UpdateStrategyKind::RollingUpdate,
            max_unavailable: 0,
            max_surge: 1,
        };
        let plan = plan_node_actions(&s, "new", &nodes).unwrap();
        assert_eq!(plan[0].1, NodeAction::Keep);
    }

    #[test]
    fn rolling_with_no_budget_anywhere_is_invalid() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "validate",
            "tenant-ds-roll-invalid"
        );
        let s = UpdateStrategy {
            kind: UpdateStrategyKind::RollingUpdate,
            max_unavailable: 0,
            max_surge: 0,
        };
        assert!(plan_node_actions(&s, "new", &[nv("n1", Some("old"), true)]).is_err());
    }

    #[test]
    fn on_delete_keeps_old_pods() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "syncDaemonSet",
            "tenant-ds-on-delete-keeps"
        );
        let plan = plan_node_actions(&on_delete(), "new", &[nv("n1", Some("old"), true)]).unwrap();
        assert_eq!(plan[0].1, NodeAction::AwaitManualDelete);
    }

    #[test]
    fn on_delete_creates_when_missing() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "syncDaemonSet",
            "tenant-ds-on-delete-create"
        );
        let plan = plan_node_actions(&on_delete(), "new", &[nv("n1", None, false)]).unwrap();
        assert_eq!(plan[0].1, NodeAction::CreatePod);
    }

    #[test]
    fn node_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/daemon/update.go",
            "NodeAction",
            "tenant-ds-action-serde"
        );
        for a in [
            NodeAction::Keep,
            NodeAction::DeletePod,
            NodeAction::CreatePod,
            NodeAction::AwaitManualDelete,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: NodeAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}

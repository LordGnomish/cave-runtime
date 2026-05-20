// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Node taints — `pkg/controller/nodelifecycle/scheduler/taint_manager.go`
//! and `pkg/util/taints/taints.go`.
//!
//! Standard well-known taints applied by the controller-manager:
//!
//! * `node.kubernetes.io/unreachable` (NoExecute) — set when a Node's
//!   Ready=Unknown for `node_monitor_grace_period`.
//! * `node.kubernetes.io/not-ready` (NoExecute) — set on Ready=False.
//! * `node.kubernetes.io/unschedulable` (NoSchedule) — mirrors `Spec.Unschedulable`.
//! * `node.kubernetes.io/out-of-service` (NoExecute) — admin-applied;
//!   triggers force-delete of pods + detach of volumes (KEP-2268).

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const TAINT_UNREACHABLE: &str = "node.kubernetes.io/unreachable";
pub const TAINT_NOT_READY: &str = "node.kubernetes.io/not-ready";
pub const TAINT_UNSCHEDULABLE: &str = "node.kubernetes.io/unschedulable";
pub const TAINT_OUT_OF_SERVICE: &str = "node.kubernetes.io/out-of-service";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: String,
    pub effect: TaintEffect,
}

impl Taint {
    pub fn new(key: impl Into<String>, effect: TaintEffect) -> Self {
        Self {
            key: key.into(),
            value: String::new(),
            effect,
        }
    }
}

/// Add `taint` to `set` if it isn't already present (key+effect match).
/// Returns true when a change is made.
pub fn add_taint(set: &mut Vec<Taint>, taint: Taint) -> bool {
    if set
        .iter()
        .any(|t| t.key == taint.key && t.effect == taint.effect)
    {
        return false;
    }
    set.push(taint);
    true
}

/// Remove every taint matching `key` (any effect).
pub fn remove_taint_by_key(set: &mut Vec<Taint>, key: &str) -> bool {
    let before = set.len();
    set.retain(|t| t.key != key);
    set.len() != before
}

/// True if `tolerations` covers `taint`. Mirrors `helper.TolerationsTolerateTaint`.
/// A toleration tolerates the taint when:
///
/// * key matches OR toleration uses the empty key with `Operator=Exists`
/// * effect matches OR toleration uses empty effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toleration {
    pub key: String,
    pub operator: TolerationOperator,
    pub value: String,
    /// `None` matches any effect.
    pub effect: Option<TaintEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TolerationOperator {
    Equal,
    Exists,
}

pub fn tolerates(tolerations: &[Toleration], taint: &Taint) -> bool {
    tolerations.iter().any(|t| {
        let key_ok = match t.operator {
            TolerationOperator::Exists if t.key.is_empty() => true,
            _ => t.key == taint.key,
        };
        let effect_ok = match t.effect {
            None => true,
            Some(e) => e == taint.effect,
        };
        let value_ok = match t.operator {
            TolerationOperator::Exists => true,
            TolerationOperator::Equal => t.value == taint.value,
        };
        key_ok && effect_ok && value_ok
    })
}

/// Returns the well-known taints the controller would apply for the
/// given Node ready / unschedulable / out-of-service signal.
pub fn desired_taints(
    ready: NodeReadyHint,
    unschedulable: bool,
    out_of_service: bool,
) -> Vec<Taint> {
    let mut out = Vec::new();
    match ready {
        NodeReadyHint::True => {}
        NodeReadyHint::False => {
            out.push(Taint::new(TAINT_NOT_READY, TaintEffect::NoExecute));
        }
        NodeReadyHint::Unknown => {
            out.push(Taint::new(TAINT_UNREACHABLE, TaintEffect::NoExecute));
        }
    }
    if unschedulable {
        out.push(Taint::new(TAINT_UNSCHEDULABLE, TaintEffect::NoSchedule));
    }
    if out_of_service {
        out.push(Taint::new(TAINT_OUT_OF_SERVICE, TaintEffect::NoExecute));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeReadyHint {
    True,
    False,
    Unknown,
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/util/taints/taints.go", "TaintsToTolerations");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn t(key: &str, effect: TaintEffect) -> Taint {
        Taint::new(key, effect)
    }

    #[test]
    fn add_taint_no_op_when_already_present() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/util/taints/taints.go",
            "AddOrUpdateTaint",
            "tenant-nl-tnt-add-dup"
        );
        let mut set = vec![t(TAINT_UNREACHABLE, TaintEffect::NoExecute)];
        assert!(!add_taint(
            &mut set,
            t(TAINT_UNREACHABLE, TaintEffect::NoExecute)
        ));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn add_taint_appends_new_effect() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/util/taints/taints.go",
            "AddOrUpdateTaint",
            "tenant-nl-tnt-add"
        );
        let mut set = vec![];
        assert!(add_taint(
            &mut set,
            t(TAINT_NOT_READY, TaintEffect::NoExecute)
        ));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn remove_taint_by_key_removes_every_effect() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/util/taints/taints.go",
            "RemoveTaint",
            "tenant-nl-tnt-remove"
        );
        let mut set = vec![
            t(TAINT_NOT_READY, TaintEffect::NoSchedule),
            t(TAINT_NOT_READY, TaintEffect::NoExecute),
            t(TAINT_UNREACHABLE, TaintEffect::NoExecute),
        ];
        assert!(remove_taint_by_key(&mut set, TAINT_NOT_READY));
        assert_eq!(set.len(), 1);
        assert_eq!(set[0].key, TAINT_UNREACHABLE);
    }

    #[test]
    fn tolerates_exact_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/helper/helpers.go",
            "TolerationsTolerateTaint",
            "tenant-nl-tlrt-equal"
        );
        let tol = Toleration {
            key: TAINT_NOT_READY.into(),
            operator: TolerationOperator::Exists,
            value: String::new(),
            effect: Some(TaintEffect::NoExecute),
        };
        assert!(tolerates(
            &[tol],
            &t(TAINT_NOT_READY, TaintEffect::NoExecute)
        ));
    }

    #[test]
    fn tolerates_with_empty_key_exists_matches_any_taint() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/helper/helpers.go",
            "TolerationsTolerateTaint",
            "tenant-nl-tlrt-wildcard"
        );
        let tol = Toleration {
            key: String::new(),
            operator: TolerationOperator::Exists,
            value: String::new(),
            effect: None,
        };
        assert!(tolerates(&[tol], &t("foo/bar", TaintEffect::NoSchedule)));
    }

    #[test]
    fn does_not_tolerate_when_effect_mismatches() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/helper/helpers.go",
            "TolerationsTolerateTaint",
            "tenant-nl-tlrt-effect-bad"
        );
        let tol = Toleration {
            key: TAINT_NOT_READY.into(),
            operator: TolerationOperator::Exists,
            value: String::new(),
            effect: Some(TaintEffect::NoSchedule),
        };
        assert!(!tolerates(
            &[tol],
            &t(TAINT_NOT_READY, TaintEffect::NoExecute)
        ));
    }

    #[test]
    fn equal_operator_requires_value_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/helper/helpers.go",
            "TolerationsTolerateTaint",
            "tenant-nl-tlrt-equal-value"
        );
        let mut taint = t("custom", TaintEffect::NoSchedule);
        taint.value = "X".into();
        let tol_match = Toleration {
            key: "custom".into(),
            operator: TolerationOperator::Equal,
            value: "X".into(),
            effect: Some(TaintEffect::NoSchedule),
        };
        let tol_miss = Toleration {
            value: "Y".into(),
            ..tol_match.clone()
        };
        assert!(tolerates(&[tol_match], &taint));
        assert!(!tolerates(&[tol_miss], &taint));
    }

    #[test]
    fn desired_taints_ready_clears_all_node_taints() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsReachable",
            "tenant-nl-tnt-desired-ready"
        );
        let want = desired_taints(NodeReadyHint::True, false, false);
        assert!(want.is_empty());
    }

    #[test]
    fn desired_taints_unknown_emits_unreachable() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsNotReady",
            "tenant-nl-tnt-desired-unknown"
        );
        let want = desired_taints(NodeReadyHint::Unknown, false, false);
        assert_eq!(want.len(), 1);
        assert_eq!(want[0].key, TAINT_UNREACHABLE);
        assert_eq!(want[0].effect, TaintEffect::NoExecute);
    }

    #[test]
    fn desired_taints_false_emits_not_ready() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsNotReady",
            "tenant-nl-tnt-desired-false"
        );
        let want = desired_taints(NodeReadyHint::False, false, false);
        assert_eq!(want[0].key, TAINT_NOT_READY);
    }

    #[test]
    fn desired_taints_unschedulable_adds_no_schedule() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "addNoScheduleTaint",
            "tenant-nl-tnt-desired-unsched"
        );
        let want = desired_taints(NodeReadyHint::True, true, false);
        assert!(
            want.iter()
                .any(|t| t.key == TAINT_UNSCHEDULABLE && t.effect == TaintEffect::NoSchedule)
        );
    }

    #[test]
    fn desired_taints_out_of_service_adds_no_execute() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "outOfServiceTaint",
            "tenant-nl-tnt-desired-oos"
        );
        let want = desired_taints(NodeReadyHint::True, false, true);
        assert!(want.iter().any(|t| t.key == TAINT_OUT_OF_SERVICE));
    }

    #[test]
    fn taint_constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/util/taints/taints.go",
            "TaintNodeUnreachable",
            "tenant-nl-tnt-const"
        );
        assert_eq!(TAINT_UNREACHABLE, "node.kubernetes.io/unreachable");
        assert_eq!(TAINT_NOT_READY, "node.kubernetes.io/not-ready");
        assert_eq!(TAINT_UNSCHEDULABLE, "node.kubernetes.io/unschedulable");
        assert_eq!(TAINT_OUT_OF_SERVICE, "node.kubernetes.io/out-of-service");
    }
}

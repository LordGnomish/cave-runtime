// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Namespace controller — `pkg/controller/namespace/namespace_controller.go`.
//!
//! Drives a Namespace through the deletion finalizer dance:
//!
//! 1. User issues `DELETE /api/v1/namespaces/<name>`.
//! 2. API server stamps `metadata.deletionTimestamp` and `status.phase = Terminating`.
//! 3. Namespace controller iterates over every monitored resource in the
//!    namespace, deleting them.
//! 4. Once empty, the controller removes the `kubernetes` finalizer from
//!    `spec.finalizers[]`. The API server then deletes the Namespace object.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespacePhase {
    Active,
    Terminating,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceState {
    pub name: String,
    pub phase: NamespacePhase,
    pub deletion_timestamp_set: bool,
    pub spec_finalizers: Vec<String>,
    pub remaining_resources: u32,
}

pub const FINALIZER_KUBERNETES: &str = "kubernetes";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespaceAction {
    /// Active namespace — nothing to do.
    NoOp,
    /// Phase isn't yet Terminating but deletion stamped → write status phase.
    SetTerminating,
    /// Phase Terminating; resources still present → continue deleting.
    DeleteResources(u32),
    /// Phase Terminating; resources empty; finalizer present → strip finalizer.
    RemoveFinalizer,
    /// Phase Terminating; resources empty; finalizer gone → API server deletes.
    AwaitDeletion,
}

pub fn evaluate(ns: &NamespaceState) -> NamespaceAction {
    if ns.phase == NamespacePhase::Active && !ns.deletion_timestamp_set {
        return NamespaceAction::NoOp;
    }
    if ns.deletion_timestamp_set && ns.phase != NamespacePhase::Terminating {
        return NamespaceAction::SetTerminating;
    }
    if ns.remaining_resources > 0 {
        return NamespaceAction::DeleteResources(ns.remaining_resources);
    }
    if ns.spec_finalizers.iter().any(|f| f == FINALIZER_KUBERNETES) {
        return NamespaceAction::RemoveFinalizer;
    }
    NamespaceAction::AwaitDeletion
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/namespace/namespace_controller.go",
    "syncNamespaceFromKey",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ns_state(
        phase: NamespacePhase,
        deletion: bool,
        fins: &[&str],
        remaining: u32,
    ) -> NamespaceState {
        NamespaceState {
            name: "demo".into(),
            phase,
            deletion_timestamp_set: deletion,
            spec_finalizers: fins.iter().map(|s| s.to_string()).collect(),
            remaining_resources: remaining,
        }
    }

    #[test]
    fn active_namespace_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/namespace/namespace_controller.go",
            "syncNamespaceFromKey",
            "tenant-ns-active"
        );
        assert_eq!(
            evaluate(&ns_state(NamespacePhase::Active, false, &[FINALIZER_KUBERNETES], 0)),
            NamespaceAction::NoOp
        );
    }

    #[test]
    fn deletion_stamped_active_phase_transitions_to_terminating() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/namespace/namespace_controller.go",
            "syncNamespaceFromKey",
            "tenant-ns-set-terminating"
        );
        assert_eq!(
            evaluate(&ns_state(NamespacePhase::Active, true, &[FINALIZER_KUBERNETES], 0)),
            NamespaceAction::SetTerminating
        );
    }

    #[test]
    fn terminating_with_resources_emits_delete() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/namespace/namespace_controller.go",
            "deleteAllContent",
            "tenant-ns-delete-content"
        );
        assert_eq!(
            evaluate(&ns_state(
                NamespacePhase::Terminating,
                true,
                &[FINALIZER_KUBERNETES],
                7
            )),
            NamespaceAction::DeleteResources(7)
        );
    }

    #[test]
    fn terminating_empty_with_finalizer_strips_it() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/namespace/namespace_controller.go",
            "finalizeNamespace",
            "tenant-ns-strip-fin"
        );
        assert_eq!(
            evaluate(&ns_state(
                NamespacePhase::Terminating,
                true,
                &[FINALIZER_KUBERNETES],
                0
            )),
            NamespaceAction::RemoveFinalizer
        );
    }

    #[test]
    fn terminating_empty_no_finalizer_awaits_deletion() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/namespace/namespace_controller.go",
            "syncNamespaceFromKey",
            "tenant-ns-await"
        );
        assert_eq!(
            evaluate(&ns_state(NamespacePhase::Terminating, true, &[], 0)),
            NamespaceAction::AwaitDeletion
        );
    }

    #[test]
    fn terminating_with_other_finalizers_only_does_not_strip_them() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/namespace/namespace_controller.go",
            "finalizeNamespace",
            "tenant-ns-other-fins"
        );
        // Only `kubernetes` is removed by the namespace controller; another
        // finalizer left behind keeps the NS waiting.
        assert_eq!(
            evaluate(&ns_state(
                NamespacePhase::Terminating,
                true,
                &["other.example.com/cleanup"],
                0
            )),
            NamespaceAction::AwaitDeletion
        );
    }

    #[test]
    fn finalizer_constant_matches_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/types.go",
            "FinalizerKubernetes",
            "tenant-ns-fin-const"
        );
        assert_eq!(FINALIZER_KUBERNETES, "kubernetes");
    }

    #[test]
    fn namespace_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/namespace/namespace_controller.go",
            "NamespaceAction",
            "tenant-ns-action-serde"
        );
        for a in [
            NamespaceAction::NoOp,
            NamespaceAction::SetTerminating,
            NamespaceAction::DeleteResources(5),
            NamespaceAction::RemoveFinalizer,
            NamespaceAction::AwaitDeletion,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: NamespaceAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}

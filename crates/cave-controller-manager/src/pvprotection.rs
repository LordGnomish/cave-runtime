// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `pvprotection` — finalizer that prevents a PersistentVolume
//! from being deleted while it's still bound or in-use.
//!
//! Mirrors `pkg/controller/volume/pvprotection/` from upstream.
//! The PVC-side protection (`pvc-protection`) is already covered
//! by cave-controller-manager's `pv` controller; this module
//! adds the *PV-side* finalizer reconciler.
//!
//! State machine:
//!
//! 1. Observer sees PV created → adds the
//!    `kubernetes.io/pv-protection` finalizer.
//! 2. Operator deletes PV → `deletionTimestamp` set, finalizer
//!    still holds.
//! 3. Reconciler checks: is PV `Bound` to a live PVC, or
//!    `Available` but referenced?
//!    * Yes → finalizer stays.
//!    * No  → finalizer removed; PV deletion proceeds.

use std::collections::BTreeMap;

/// PV phase as the apiserver reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PvPhase {
    Available,
    Bound,
    Released,
    Failed,
    Pending,
}

/// One PersistentVolume observed by the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPv {
    pub name: String,
    pub phase: PvPhase,
    /// `Some(uid)` if a PVC is bound to this PV; `None` else.
    pub claim_ref_uid: Option<String>,
    /// `Some(when)` if a deletion has been requested. The
    /// finalizer reconciler only acts when this is set.
    pub deletion_unix: Option<i64>,
    /// Whether the `kubernetes.io/pv-protection` finalizer is
    /// currently on the PV.
    pub has_finalizer: bool,
}

/// What the reconciler decides for one PV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// PV not flagged for deletion — finalizer should be
    /// present.
    EnsureFinalizer,
    /// PV deletion requested AND PV is still in-use; finalizer
    /// stays.
    BlockDeletion,
    /// PV deletion requested AND PV is no longer in-use;
    /// finalizer should be removed.
    ReleaseFinalizer,
    /// No-op (PV already in the desired terminal state).
    NoOp,
}

/// Decide what to do with one PV. Pure-function — every input
/// determines the output, no side-effects.
pub fn evaluate(pv: &ObservedPv) -> Action {
    match pv.deletion_unix {
        None => {
            if pv.has_finalizer {
                Action::NoOp
            } else {
                Action::EnsureFinalizer
            }
        }
        Some(_) => {
            let in_use = matches!(pv.phase, PvPhase::Bound) || pv.claim_ref_uid.is_some();
            if in_use {
                Action::BlockDeletion
            } else if pv.has_finalizer {
                Action::ReleaseFinalizer
            } else {
                Action::NoOp
            }
        }
    }
}

/// Per-controller state: in-flight evaluation history per PV
/// name. Useful for tests + admin UI metrics.
#[derive(Debug, Default)]
pub struct PvProtector {
    last_actions: BTreeMap<String, Action>,
}

impl PvProtector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reconcile(&mut self, pv: &ObservedPv) -> Action {
        let action = evaluate(pv);
        self.last_actions.insert(pv.name.clone(), action);
        action
    }

    pub fn last_action(&self, pv_name: &str) -> Option<Action> {
        self.last_actions.get(pv_name).copied()
    }

    pub fn forget(&mut self, pv_name: &str) {
        self.last_actions.remove(pv_name);
    }

    pub fn tracked_count(&self) -> usize {
        self.last_actions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pv(name: &str) -> ObservedPv {
        ObservedPv {
            name: name.into(),
            phase: PvPhase::Available,
            claim_ref_uid: None,
            deletion_unix: None,
            has_finalizer: false,
        }
    }

    #[test]
    fn newly_observed_pv_gets_finalizer() {
        let p = pv("a");
        assert_eq!(evaluate(&p), Action::EnsureFinalizer);
    }

    #[test]
    fn already_finalised_pv_no_op() {
        let mut p = pv("a");
        p.has_finalizer = true;
        assert_eq!(evaluate(&p), Action::NoOp);
    }

    #[test]
    fn bound_pv_under_deletion_blocks() {
        let mut p = pv("a");
        p.has_finalizer = true;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Bound;
        p.claim_ref_uid = Some("uid".into());
        assert_eq!(evaluate(&p), Action::BlockDeletion);
    }

    #[test]
    fn released_unbound_pv_release_finalizer() {
        let mut p = pv("a");
        p.has_finalizer = true;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Released;
        p.claim_ref_uid = None;
        assert_eq!(evaluate(&p), Action::ReleaseFinalizer);
    }

    #[test]
    fn available_unclaimed_pv_under_deletion_releases() {
        let mut p = pv("a");
        p.has_finalizer = true;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Available;
        assert_eq!(evaluate(&p), Action::ReleaseFinalizer);
    }

    #[test]
    fn pv_under_deletion_without_finalizer_is_noop() {
        let mut p = pv("a");
        p.has_finalizer = false;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Released;
        assert_eq!(evaluate(&p), Action::NoOp);
    }

    #[test]
    fn bound_phase_alone_keeps_finalizer_on_delete() {
        let mut p = pv("a");
        p.has_finalizer = true;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Bound;
        p.claim_ref_uid = None;
        assert_eq!(evaluate(&p), Action::BlockDeletion);
    }

    #[test]
    fn claim_ref_alone_keeps_finalizer_on_delete() {
        let mut p = pv("a");
        p.has_finalizer = true;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Available;
        p.claim_ref_uid = Some("uid".into());
        assert_eq!(evaluate(&p), Action::BlockDeletion);
    }

    #[test]
    fn reconcile_records_last_action() {
        let mut p = PvProtector::new();
        let pv1 = pv("alpha");
        let a = p.reconcile(&pv1);
        assert_eq!(p.last_action("alpha"), Some(a));
        assert_eq!(p.tracked_count(), 1);
    }

    #[test]
    fn forget_drops_tracked_entry() {
        let mut p = PvProtector::new();
        p.reconcile(&pv("alpha"));
        p.forget("alpha");
        assert!(p.last_action("alpha").is_none());
    }

    #[test]
    fn failed_phase_treated_as_in_use_when_claim_ref_present() {
        let mut p = pv("a");
        p.has_finalizer = true;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Failed;
        p.claim_ref_uid = Some("u".into());
        assert_eq!(evaluate(&p), Action::BlockDeletion);
    }

    #[test]
    fn pending_phase_no_claim_ref_releases_on_delete() {
        let mut p = pv("a");
        p.has_finalizer = true;
        p.deletion_unix = Some(1000);
        p.phase = PvPhase::Pending;
        p.claim_ref_uid = None;
        assert_eq!(evaluate(&p), Action::ReleaseFinalizer);
    }
}

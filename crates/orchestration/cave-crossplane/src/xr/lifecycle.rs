// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composite Resource lifecycle FSM.
//!
//! Upstream: internal/controller/apiextensions/composite/reconciler.go

use crate::models::DeletionPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XrPhase {
    Pending,
    Creating,
    Ready,
    Updating,
    Deleting,
    Failed,
}

impl XrPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            XrPhase::Pending => "Pending",
            XrPhase::Creating => "Creating",
            XrPhase::Ready => "Ready",
            XrPhase::Updating => "Updating",
            XrPhase::Deleting => "Deleting",
            XrPhase::Failed => "Failed",
        }
    }

    /// Determine the next legal phase given an `event`. Returns None when the
    /// transition is illegal.
    pub fn next(self, ev: XrEvent) -> Option<XrPhase> {
        use XrEvent::*;
        use XrPhase::*;
        let next = match (self, ev) {
            (Pending, ComposeStarted) => Creating,
            (Creating, ComposeReady) => Ready,
            (Creating, ComposeFailed) => Failed,
            (Ready, SpecChanged) => Updating,
            (Updating, ComposeReady) => Ready,
            (Updating, ComposeFailed) => Failed,
            (Ready, DeletionRequested) | (Updating, DeletionRequested) => Deleting,
            (Pending, DeletionRequested) | (Creating, DeletionRequested) => Deleting,
            (Deleting, FinalizerCleared) => Pending, // resurrection guarded by reconciler
            (Failed, ComposeStarted) => Creating,
            _ => return None,
        };
        Some(next)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrEvent {
    ComposeStarted,
    ComposeReady,
    ComposeFailed,
    SpecChanged,
    DeletionRequested,
    FinalizerCleared,
}

/// Finalizer name written onto XRs to gate deletion until cleanup completes.
pub const XR_FINALIZER: &str = "composite.apiextensions.crossplane.io/composed";

/// Compute the deletion plan given a policy and the count of composed resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionPlan {
    pub policy: PlanPolicy,
    pub composed_to_delete: usize,
    pub composed_to_orphan: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanPolicy {
    Delete,
    Orphan,
}

pub fn plan_deletion(policy: DeletionPolicy, composed_count: usize) -> DeletionPlan {
    match policy {
        DeletionPolicy::Delete => DeletionPlan {
            policy: PlanPolicy::Delete,
            composed_to_delete: composed_count,
            composed_to_orphan: 0,
        },
        DeletionPolicy::Orphan => DeletionPlan {
            policy: PlanPolicy::Orphan,
            composed_to_delete: 0,
            composed_to_orphan: composed_count,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_str_names_match() {
        assert_eq!(XrPhase::Ready.as_str(), "Ready");
        assert_eq!(XrPhase::Pending.as_str(), "Pending");
    }

    #[test]
    fn pending_to_creating_legal() {
        assert_eq!(
            XrPhase::Pending.next(XrEvent::ComposeStarted),
            Some(XrPhase::Creating)
        );
    }

    #[test]
    fn creating_to_ready_legal() {
        assert_eq!(
            XrPhase::Creating.next(XrEvent::ComposeReady),
            Some(XrPhase::Ready)
        );
    }

    #[test]
    fn ready_to_updating_then_ready() {
        let p = XrPhase::Ready.next(XrEvent::SpecChanged).unwrap();
        assert_eq!(p, XrPhase::Updating);
        let p2 = p.next(XrEvent::ComposeReady).unwrap();
        assert_eq!(p2, XrPhase::Ready);
    }

    #[test]
    fn illegal_transition_returns_none() {
        assert!(XrPhase::Ready.next(XrEvent::FinalizerCleared).is_none());
    }

    #[test]
    fn deletion_from_any_active_phase() {
        for p in [XrPhase::Pending, XrPhase::Creating, XrPhase::Ready, XrPhase::Updating] {
            assert_eq!(p.next(XrEvent::DeletionRequested), Some(XrPhase::Deleting));
        }
    }

    #[test]
    fn failed_can_resume() {
        assert_eq!(
            XrPhase::Failed.next(XrEvent::ComposeStarted),
            Some(XrPhase::Creating)
        );
    }

    #[test]
    fn deletion_plan_delete() {
        let p = plan_deletion(DeletionPolicy::Delete, 3);
        assert_eq!(p.composed_to_delete, 3);
        assert_eq!(p.composed_to_orphan, 0);
    }

    #[test]
    fn deletion_plan_orphan() {
        let p = plan_deletion(DeletionPolicy::Orphan, 5);
        assert_eq!(p.composed_to_orphan, 5);
        assert_eq!(p.composed_to_delete, 0);
    }

    #[test]
    fn finalizer_name_stable() {
        assert!(XR_FINALIZER.contains("crossplane.io"));
    }
}

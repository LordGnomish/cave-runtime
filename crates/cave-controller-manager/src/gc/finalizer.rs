// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GC finalizer string handling — `pkg/controller/garbagecollector/finalizer.go`.
//!
//! Finalizers in `metadata.finalizers[]`:
//!
//! * `foregroundDeletion` — set by the API server on DELETE with
//!   propagationPolicy=Foreground; controller-manager removes it after
//!   blocking dependents are gone.
//! * `orphan` — set on DELETE with propagationPolicy=Orphan;
//!   controller-manager removes it after rewriting dependent owner refs.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const FINALIZER_FOREGROUND_DELETION: &str = "foregroundDeletion";
pub const FINALIZER_ORPHAN: &str = "orphan";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinalizerOp {
    Add(&'static str),
    Remove(&'static str),
    NoOp,
}

/// Decide which finalizer to add when a DELETE arrives. The behavior in
/// `pkg/registry/garbagecollector/operations.go::DeletionFinalizersForGarbageCollection`.
pub fn add_for_propagation(
    propagation: &str,
    existing: &[String],
) -> FinalizerOp {
    match propagation {
        "Foreground" => {
            if existing.iter().any(|f| f == FINALIZER_FOREGROUND_DELETION) {
                FinalizerOp::NoOp
            } else {
                FinalizerOp::Add(FINALIZER_FOREGROUND_DELETION)
            }
        }
        "Orphan" => {
            if existing.iter().any(|f| f == FINALIZER_ORPHAN) {
                FinalizerOp::NoOp
            } else {
                FinalizerOp::Add(FINALIZER_ORPHAN)
            }
        }
        _ => FinalizerOp::NoOp,
    }
}

/// Remove the finalizer indicated by the propagation flow once the
/// associated work has been observed-complete.
pub fn remove_when_complete(
    propagation: &str,
    blocked: bool,
    existing: &[String],
) -> FinalizerOp {
    if blocked {
        return FinalizerOp::NoOp;
    }
    let target = match propagation {
        "Foreground" => FINALIZER_FOREGROUND_DELETION,
        "Orphan" => FINALIZER_ORPHAN,
        _ => return FinalizerOp::NoOp,
    };
    if existing.iter().any(|f| f == target) {
        FinalizerOp::Remove(target)
    } else {
        FinalizerOp::NoOp
    }
}

/// Filter a finalizer list to remove the named finalizer (single-pass).
pub fn strip(finalizers: &[String], name: &str) -> Vec<String> {
    finalizers.iter().filter(|f| f.as_str() != name).cloned().collect()
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/garbagecollector/finalizer.go",
    "Finalizers",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn add_foreground_when_missing() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/garbagecollector/operations.go",
            "DeletionFinalizersForGarbageCollection",
            "tenant-gc-fin-add-fg"
        );
        assert_eq!(
            add_for_propagation("Foreground", &[]),
            FinalizerOp::Add(FINALIZER_FOREGROUND_DELETION)
        );
    }

    #[test]
    fn add_orphan_when_missing() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/garbagecollector/operations.go",
            "DeletionFinalizersForGarbageCollection",
            "tenant-gc-fin-add-orph"
        );
        assert_eq!(
            add_for_propagation("Orphan", &[]),
            FinalizerOp::Add(FINALIZER_ORPHAN)
        );
    }

    #[test]
    fn add_is_noop_when_already_present() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/garbagecollector/operations.go",
            "DeletionFinalizersForGarbageCollection",
            "tenant-gc-fin-add-dup"
        );
        let existing = vec![FINALIZER_FOREGROUND_DELETION.to_string()];
        assert_eq!(add_for_propagation("Foreground", &existing), FinalizerOp::NoOp);
    }

    #[test]
    fn add_unknown_propagation_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/garbagecollector/operations.go",
            "DeletionFinalizersForGarbageCollection",
            "tenant-gc-fin-add-unknown"
        );
        assert_eq!(add_for_propagation("Background", &[]), FinalizerOp::NoOp);
    }

    #[test]
    fn remove_blocked_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/finalizer.go",
            "removeFinalizer",
            "tenant-gc-fin-blocked"
        );
        let existing = vec![FINALIZER_FOREGROUND_DELETION.to_string()];
        assert_eq!(
            remove_when_complete("Foreground", true, &existing),
            FinalizerOp::NoOp
        );
    }

    #[test]
    fn remove_unblocked_emits_remove() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/finalizer.go",
            "removeFinalizer",
            "tenant-gc-fin-unblocked"
        );
        let existing = vec![FINALIZER_FOREGROUND_DELETION.to_string()];
        assert_eq!(
            remove_when_complete("Foreground", false, &existing),
            FinalizerOp::Remove(FINALIZER_FOREGROUND_DELETION)
        );
    }

    #[test]
    fn remove_when_finalizer_missing_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/finalizer.go",
            "removeFinalizer",
            "tenant-gc-fin-no-fin"
        );
        assert_eq!(
            remove_when_complete("Foreground", false, &[]),
            FinalizerOp::NoOp
        );
    }

    #[test]
    fn strip_removes_single_finalizer() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/finalizer.go",
            "removeFinalizer",
            "tenant-gc-fin-strip"
        );
        let f = vec![
            FINALIZER_FOREGROUND_DELETION.to_string(),
            "kubernetes.io/pvc-protection".to_string(),
        ];
        let after = strip(&f, FINALIZER_FOREGROUND_DELETION);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0], "kubernetes.io/pvc-protection");
    }

    #[test]
    fn finalizer_constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/meta/v1/types.go",
            "FinalizerOrphanDependents",
            "tenant-gc-fin-const"
        );
        assert_eq!(FINALIZER_FOREGROUND_DELETION, "foregroundDeletion");
        assert_eq!(FINALIZER_ORPHAN, "orphan");
    }
}

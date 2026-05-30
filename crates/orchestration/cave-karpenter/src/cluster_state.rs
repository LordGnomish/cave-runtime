// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Cluster state — port of the node-nomination and deletion-marking surface in
//! `pkg/controllers/state/cluster.go` from kubernetes-sigs/karpenter v1.12.1
//! (sha `ed490e8`), plus the disruption candidate gate from
//! `pkg/controllers/disruption/candidate.go`.
//!
//! When the provisioning scheduler decides a pending pod should land on a
//! particular node it *nominates* that node for a short window. A nominated
//! node must not be torn down by the disruption controller in the meantime, or
//! the placement it just computed would be undone. Independently, a node the
//! controller has already decided to remove is *marked for deletion* and is
//! likewise excluded from the candidate set.
//!
//! cave keys both caches on the node / NodeClaim name (the
//! [`crate::disruption::Decision::claim_name`]). Time is threaded explicitly so
//! the logic stays pure and deterministic.

use crate::disruption::Decision;
use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

/// In-memory cluster bookkeeping for nomination + deletion state.
#[derive(Debug, Default, Clone)]
pub struct ClusterState {
    /// node → nomination expiry (`IsNodeNominated` is true while `now < expiry`).
    nominations: BTreeMap<String, SystemTime>,
    /// nodes the disruption controller has committed to removing.
    marked_for_deletion: BTreeSet<String>,
}

impl ClusterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// `NominateNodeForPod` — protect `node` from disruption until `until`.
    /// Re-nominating overwrites the expiry (windows extend, never shrink the
    /// caller's intent).
    pub fn nominate(&mut self, node: &str, until: SystemTime) {
        self.nominations.insert(node.to_string(), until);
    }

    /// `IsNodeNominated` — true while the nomination window is still open.
    pub fn is_nominated(&self, node: &str, now: SystemTime) -> bool {
        match self.nominations.get(node) {
            Some(expiry) => now < *expiry,
            None => false,
        }
    }

    /// `MarkForDeletion`.
    pub fn mark_for_deletion(&mut self, node: &str) {
        self.marked_for_deletion.insert(node.to_string());
    }

    /// `UnmarkForDeletion`.
    pub fn unmark_for_deletion(&mut self, node: &str) {
        self.marked_for_deletion.remove(node);
    }

    /// `StateNode.MarkedForDeletion`.
    pub fn is_marked_for_deletion(&self, node: &str) -> bool {
        self.marked_for_deletion.contains(node)
    }

    /// A node is a disruption candidate only if it is neither nominated
    /// (within window) nor already marked for deletion. Mirrors the guards in
    /// `disruption/candidate.go::NewCandidate`.
    pub fn is_disruption_candidate(&self, node: &str, now: SystemTime) -> bool {
        !self.is_nominated(node, now) && !self.is_marked_for_deletion(node)
    }
}

/// Drop disruption [`Decision`]s targeting nodes that are not currently
/// disruptable (nominated within window or marked for deletion). The order of
/// the surviving decisions is preserved.
pub fn filter_disruptable(
    decisions: Vec<Decision>,
    state: &ClusterState,
    now: SystemTime,
) -> Vec<Decision> {
    decisions
        .into_iter()
        .filter(|d| state.is_disruption_candidate(&d.claim_name, now))
        .collect()
}

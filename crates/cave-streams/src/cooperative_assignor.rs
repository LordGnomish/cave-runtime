// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/java/org/apache/kafka/clients/consumer/internals/CooperativeStickyAssignor.java
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2 clients/src/main/java/org/apache/kafka/clients/consumer/internals/AbstractStickyAssignor.java

//! Cooperative incremental rebalance — KIP-415 (Connect rolling-bounce
//! upgrade) and KIP-429 (cooperative-sticky for the consumer group).
//!
//! The two-phase plan in [`crate::incremental_rebalance`] computes
//! per-member "release then assign" diffs from a *given* target.
//! This module provides the **assignor** that *produces* that target.
//!
//! ## IncrementalAssignor contract (KIP-415)
//!
//! `assign(previous, members, partitions) -> target` such that:
//!
//! 1. `target` is balanced — every member gets `floor(P/M)` or
//!    `ceil(P/M)` partitions (no member is more than one above the
//!    minimum).
//! 2. `target` is sticky — if a member owned a partition in
//!    `previous` and is still in `members`, the partition stays put
//!    unless balance dictates otherwise.
//! 3. The **revoke-then-assign** workflow — a partition that needs
//!    to move is *first* revoked from its old owner (Phase 1) and
//!    *then* assigned to its new owner (Phase 2). The plan is
//!    materialised by feeding `assign`'s output into
//!    [`crate::incremental_rebalance::compute_incremental_plan`].
//!
//! ## CooperativeStickyAssignor parity
//!
//! Upstream's `CooperativeStickyAssignor` extends
//! `AbstractStickyAssignor` and overrides `onAssignment` to
//! return the *target* rather than directly mutating the
//! consumer's owned set. Our [`CooperativeStickyAssignor`]
//! mirrors that: `assign` returns the same balanced+sticky
//! target every time.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::incremental_rebalance::{
    compute_incremental_plan, IncrementalRebalancePlan, Tp,
};

/// The trait the cooperative path uses to compute the *next*
/// assignment. Mirrors upstream `ConsumerPartitionAssignor`.
pub trait IncrementalAssignor {
    /// Compute a balanced + sticky target for `members` to own
    /// `partitions`, given each member's `previous` assignment.
    fn assign(
        &self,
        previous: &HashMap<String, BTreeSet<Tp>>,
        members: &[String],
        partitions: &BTreeSet<Tp>,
    ) -> HashMap<String, BTreeSet<Tp>>;

    /// Drive a full cooperative cycle: compute target, diff
    /// against `previous`, return the two-phase plan.
    fn cooperative_plan(
        &self,
        previous: &HashMap<String, BTreeSet<Tp>>,
        members: &[String],
        partitions: &BTreeSet<Tp>,
    ) -> IncrementalRebalancePlan {
        let target = self.assign(previous, members, partitions);
        compute_incremental_plan(previous, &target)
    }

    /// Human-readable protocol name — matches upstream's
    /// `ConsumerPartitionAssignor.name()`.
    fn name(&self) -> &'static str;
}

/// `CooperativeStickyAssignor` — same balanced+sticky logic as
/// the eager `StickyAssignor` but the revoke-then-assign discipline
/// is enforced by [`IncrementalAssignor::cooperative_plan`].
#[derive(Debug, Clone, Default)]
pub struct CooperativeStickyAssignor;

impl CooperativeStickyAssignor {
    pub fn new() -> Self {
        Self
    }
}

impl IncrementalAssignor for CooperativeStickyAssignor {
    fn name(&self) -> &'static str {
        "cooperative-sticky"
    }

    fn assign(
        &self,
        previous: &HashMap<String, BTreeSet<Tp>>,
        members: &[String],
        partitions: &BTreeSet<Tp>,
    ) -> HashMap<String, BTreeSet<Tp>> {
        sticky_balanced_assign(previous, members, partitions)
    }
}

/// Pure helper — computes a balanced+sticky assignment.
///
/// Algorithm (mirrors `AbstractStickyAssignor.balance()`):
///
/// 1. Determine the target size per member: `floor(P/M)` for the
///    last `(P mod M)` members, `ceil(P/M)` for the first ones (sorted
///    lexicographically, matching upstream's deterministic ordering).
/// 2. Seed each member's bucket with the partitions it owned
///    previously *that are still in `partitions` AND that we can
///    still afford* under the per-member cap.
/// 3. Distribute the unassigned partitions to members in
///    round-robin order, respecting the cap.
fn sticky_balanced_assign(
    previous: &HashMap<String, BTreeSet<Tp>>,
    members: &[String],
    partitions: &BTreeSet<Tp>,
) -> HashMap<String, BTreeSet<Tp>> {
    let mut sorted_members: Vec<String> = members.to_vec();
    sorted_members.sort();
    let mut out: BTreeMap<String, BTreeSet<Tp>> = BTreeMap::new();
    for m in &sorted_members {
        out.insert(m.clone(), BTreeSet::new());
    }
    if sorted_members.is_empty() || partitions.is_empty() {
        return out.into_iter().collect();
    }
    let m_count = sorted_members.len();
    let p_count = partitions.len();
    let base = p_count / m_count;
    let extra = p_count % m_count;
    // First `extra` (in sorted order) get `base+1`; rest get `base`.
    let cap_for = |idx: usize| if idx < extra { base + 1 } else { base };

    // Step 1: keep partitions a member already owns, capped.
    let mut assigned: BTreeSet<Tp> = BTreeSet::new();
    for (idx, member) in sorted_members.iter().enumerate() {
        let cap = cap_for(idx);
        if cap == 0 {
            continue;
        }
        let prev = match previous.get(member) {
            Some(s) => s,
            None => continue,
        };
        let mut kept: Vec<Tp> = prev
            .iter()
            .filter(|tp| partitions.contains(tp))
            .cloned()
            .collect();
        kept.sort();
        for tp in kept.into_iter().take(cap) {
            if assigned.insert(tp.clone()) {
                out.get_mut(member).unwrap().insert(tp);
            }
        }
    }

    // Step 2: round-robin the remaining partitions across
    // under-capped members.
    let mut unassigned: Vec<Tp> =
        partitions.iter().filter(|tp| !assigned.contains(tp)).cloned().collect();
    unassigned.sort();
    let mut cursor = 0usize;
    for tp in unassigned {
        let mut placed = false;
        for _ in 0..m_count {
            let idx = cursor % m_count;
            cursor += 1;
            let member = &sorted_members[idx];
            let cap = cap_for(idx);
            let owned = out.get(member).map(|s| s.len()).unwrap_or(0);
            if owned < cap {
                out.get_mut(member).unwrap().insert(tp);
                placed = true;
                break;
            }
        }
        // The algorithm guarantees every partition fits — assert
        // for safety; if it ever fires, the caller passed an
        // inconsistent (members, partitions) pair.
        debug_assert!(placed, "balanced assign failed to place partition");
    }
    out.into_iter().collect()
}

/// Free-function wrapper to keep call-sites short.
pub fn cooperative_sticky_plan(
    previous: &HashMap<String, BTreeSet<Tp>>,
    members: &[String],
    partitions: &BTreeSet<Tp>,
) -> IncrementalRebalancePlan {
    CooperativeStickyAssignor::new().cooperative_plan(previous, members, partitions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::incremental_rebalance::is_already_balanced;

    fn tp(topic: &str, p: i32) -> Tp {
        (topic.to_string(), p)
    }

    #[test]
    fn assign_empty_inputs_yields_empty() {
        let a = CooperativeStickyAssignor::new();
        let prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        let parts: BTreeSet<Tp> = BTreeSet::new();
        let out = a.assign(&prev, &[], &parts);
        assert!(out.is_empty());
    }

    #[test]
    fn assign_no_previous_distributes_round_robin() {
        let a = CooperativeStickyAssignor::new();
        let prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        let members = vec!["a".to_string(), "b".to_string()];
        let parts: BTreeSet<Tp> =
            [tp("t", 0), tp("t", 1), tp("t", 2), tp("t", 3)].into_iter().collect();
        let out = a.assign(&prev, &members, &parts);
        assert_eq!(out["a"].len(), 2);
        assert_eq!(out["b"].len(), 2);
        // Union of all owned == partitions; nothing duplicated.
        let union: BTreeSet<Tp> =
            out.values().flat_map(|s| s.iter().cloned()).collect();
        assert_eq!(union, parts);
    }

    #[test]
    fn assign_sticky_keeps_partitions_when_balance_allows() {
        let a = CooperativeStickyAssignor::new();
        let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        prev.insert("a".into(), [tp("t", 0), tp("t", 1)].into_iter().collect());
        prev.insert("b".into(), [tp("t", 2), tp("t", 3)].into_iter().collect());
        let members = vec!["a".to_string(), "b".to_string()];
        let parts: BTreeSet<Tp> =
            [tp("t", 0), tp("t", 1), tp("t", 2), tp("t", 3)].into_iter().collect();
        let out = a.assign(&prev, &members, &parts);
        assert_eq!(out["a"], prev["a"]);
        assert_eq!(out["b"], prev["b"]);
    }

    #[test]
    fn assign_new_member_steals_balanced_share() {
        let a = CooperativeStickyAssignor::new();
        let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        prev.insert(
            "a".into(),
            [tp("t", 0), tp("t", 1), tp("t", 2), tp("t", 3)].into_iter().collect(),
        );
        let members = vec!["a".to_string(), "b".to_string()];
        let parts: BTreeSet<Tp> =
            [tp("t", 0), tp("t", 1), tp("t", 2), tp("t", 3)].into_iter().collect();
        let out = a.assign(&prev, &members, &parts);
        assert_eq!(out["a"].len(), 2);
        assert_eq!(out["b"].len(), 2);
    }

    #[test]
    fn assign_uneven_split_capped_at_ceil() {
        // 5 partitions, 2 members → caps {3, 2}.
        let a = CooperativeStickyAssignor::new();
        let prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        let members = vec!["a".to_string(), "b".to_string()];
        let parts: BTreeSet<Tp> = (0..5).map(|p| tp("t", p)).collect();
        let out = a.assign(&prev, &members, &parts);
        assert_eq!(out["a"].len(), 3);
        assert_eq!(out["b"].len(), 2);
    }

    #[test]
    fn cooperative_plan_revoke_then_assign() {
        let a = CooperativeStickyAssignor::new();
        let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        prev.insert(
            "a".into(),
            [tp("t", 0), tp("t", 1), tp("t", 2), tp("t", 3)].into_iter().collect(),
        );
        let members = vec!["a".to_string(), "b".to_string()];
        let parts: BTreeSet<Tp> =
            [tp("t", 0), tp("t", 1), tp("t", 2), tp("t", 3)].into_iter().collect();
        let plan = a.cooperative_plan(&prev, &members, &parts);
        // Member a must release exactly 2 partitions.
        let a_phase1 = plan.phase1.iter().find(|m| m.member_id == "a").unwrap();
        assert_eq!(a_phase1.to_release.len(), 2);
        assert_eq!(a_phase1.retain.len(), 2);
        // Phase 2 grants both members their target.
        let a_phase2 = plan.phase2.iter().find(|m| m.member_id == "a").unwrap();
        let b_phase2 = plan.phase2.iter().find(|m| m.member_id == "b").unwrap();
        assert_eq!(a_phase2.assigned.len(), 2);
        assert_eq!(b_phase2.assigned.len(), 2);
        assert!(!is_already_balanced(&plan));
    }

    #[test]
    fn cooperative_plan_no_op_when_balanced() {
        let a = CooperativeStickyAssignor::new();
        let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        prev.insert("a".into(), [tp("t", 0)].into_iter().collect());
        prev.insert("b".into(), [tp("t", 1)].into_iter().collect());
        let members = vec!["a".to_string(), "b".to_string()];
        let parts: BTreeSet<Tp> = [tp("t", 0), tp("t", 1)].into_iter().collect();
        let plan = a.cooperative_plan(&prev, &members, &parts);
        assert!(is_already_balanced(&plan));
        assert_eq!(plan.released_count, 0);
    }

    #[test]
    fn cooperative_plan_handles_member_departure() {
        let a = CooperativeStickyAssignor::new();
        let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        prev.insert("a".into(), [tp("t", 0), tp("t", 1)].into_iter().collect());
        prev.insert("b".into(), [tp("t", 2), tp("t", 3)].into_iter().collect());
        // a leaves the group.
        let members = vec!["b".to_string()];
        let parts: BTreeSet<Tp> =
            [tp("t", 0), tp("t", 1), tp("t", 2), tp("t", 3)].into_iter().collect();
        let plan = a.cooperative_plan(&prev, &members, &parts);
        // a's phase1 must release everything.
        let a_phase1 = plan.phase1.iter().find(|m| m.member_id == "a").unwrap();
        assert!(a_phase1.retain.is_empty());
        assert_eq!(a_phase1.to_release.len(), 2);
        // b ends up with all 4 partitions.
        let b_phase2 = plan.phase2.iter().find(|m| m.member_id == "b").unwrap();
        assert_eq!(b_phase2.assigned.len(), 4);
    }

    #[test]
    fn assign_drops_partitions_no_longer_present() {
        // Partition (t,5) was previously owned by a but isn't in
        // the target partitions set anymore (topic shrank). Must
        // not leak into the assignment.
        let a = CooperativeStickyAssignor::new();
        let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        prev.insert(
            "a".into(),
            [tp("t", 0), tp("t", 5)].into_iter().collect(),
        );
        let members = vec!["a".to_string()];
        let parts: BTreeSet<Tp> = [tp("t", 0)].into_iter().collect();
        let out = a.assign(&prev, &members, &parts);
        assert_eq!(out["a"], parts);
    }

    #[test]
    fn name_returns_cooperative_sticky() {
        assert_eq!(CooperativeStickyAssignor::new().name(), "cooperative-sticky");
    }

    #[test]
    fn cooperative_sticky_plan_freefn_matches_struct() {
        let a = CooperativeStickyAssignor::new();
        let mut prev: HashMap<String, BTreeSet<Tp>> = HashMap::new();
        prev.insert("a".into(), [tp("t", 0)].into_iter().collect());
        let members = vec!["a".to_string(), "b".to_string()];
        let parts: BTreeSet<Tp> = [tp("t", 0), tp("t", 1)].into_iter().collect();
        let p1 = cooperative_sticky_plan(&prev, &members, &parts);
        let p2 = a.cooperative_plan(&prev, &members, &parts);
        assert_eq!(p1, p2);
    }
}

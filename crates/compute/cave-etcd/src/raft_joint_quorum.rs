// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Joint-consensus committed-index calculation.
//!
//! Mirrors etcd v3.6 `raft/quorum/joint.go` (`JointConfig.CommittedIndex`)
//! and `raft/quorum/majority.go` (`MajorityConfig.CommittedIndex`).
//!
//! # Algorithm
//!
//! During a joint configuration change etcd requires a log entry to be
//! acknowledged by a **strict majority of BOTH** the outgoing config
//! (C_old) and the incoming config (C_new) before it may be committed.
//! The committed index is therefore the **minimum** of two per-config
//! majority-committed indices.
//!
//! Per-config majority-committed index (MajorityConfig.CommittedIndex):
//!
//! Given a set of voter IDs and a map of `member_id → match_index`,
//!
//! 1. Collect the match index for every voter (0 for any absent voter).
//! 2. Sort the collected indices in *descending* order.
//! 3. The majority quorum size for `n` voters is `n/2 + 1`.
//! 4. The committed index is `sorted[quorum - 1]` (0-based), i.e. the
//!    lowest index held by at least a quorum of voters.
//!
//! Special cases:
//! * Empty voter set → `u64::MAX` (vacuously committed).
//!
//! # References
//!
//! * <https://github.com/etcd-io/etcd/blob/v3.6.10/raft/quorum/joint.go>
//! * <https://github.com/etcd-io/etcd/blob/v3.6.10/raft/quorum/majority.go>
//! * Ongaro & Ousterhout §4.3 "Cluster membership changes"

use std::collections::HashMap;

/// Compute the committed index for a single majority configuration.
///
/// # Parameters
/// * `voters`    — slice of voter IDs in this config (may overlap with
///                 another config in joint mode).
/// * `match_idx` — map of `member_id → highest log index acknowledged`.
///                 Members absent from the map are assumed to have acked
///                 nothing (index 0).
///
/// # Returns
/// The highest log index that at least a strict majority of `voters` have
/// acknowledged. Returns `u64::MAX` when `voters` is empty.
pub fn majority_committed_index(voters: &[u64], match_idx: &HashMap<u64, u64>) -> u64 {
    if voters.is_empty() {
        // Vacuous: no voter can block commitment.
        return u64::MAX;
    }

    // Collect match indices, defaulting absent members to 0.
    let mut indices: Vec<u64> = voters
        .iter()
        .map(|id| *match_idx.get(id).unwrap_or(&0))
        .collect();

    // Sort descending so the element at position [quorum-1] is the
    // smallest index held by at least `quorum` members.
    indices.sort_unstable_by(|a, b| b.cmp(a));

    // Strict majority: n/2 + 1.  For n voters the quorum-th largest
    // (1-indexed) is at position [quorum-1] (0-indexed).
    let quorum = voters.len() / 2 + 1;

    // quorum is always <= voters.len() because len/2+1 <= len for len >= 1.
    indices[quorum - 1]
}

/// Compute the joint committed index for a joint consensus transition.
///
/// An entry is committed only when it is committed in **both** the
/// outgoing (`c_old`) and the incoming (`c_new`) voter sets.
///
/// # Parameters
/// * `c_old`     — voters in the outgoing configuration.
/// * `c_new`     — voters in the incoming configuration.
/// * `match_idx` — map of `member_id → highest acknowledged log index`.
///
/// # Returns
/// `min(committed_in(c_old), committed_in(c_new))`.
///
/// When `c_old` is empty the outgoing config is considered vacuously
/// committed at `u64::MAX`, so the result equals `committed_in(c_new)`.
/// When both are empty the result is `u64::MAX`.
pub fn joint_committed_index(
    c_old: &[u64],
    c_new: &[u64],
    match_idx: &HashMap<u64, u64>,
) -> u64 {
    let committed_old = majority_committed_index(c_old, match_idx);
    let committed_new = majority_committed_index(c_new, match_idx);
    committed_old.min(committed_new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majority_quorum_single_voter() {
        let acks: HashMap<u64, u64> = [(1, 99)].into_iter().collect();
        assert_eq!(majority_committed_index(&[1], &acks), 99);
    }

    #[test]
    fn majority_quorum_three_voters_all_present() {
        // Sorted desc: [30,20,10] → quorum=2 → index[1] = 20.
        let acks: HashMap<u64, u64> = [(1, 30), (2, 20), (3, 10)].into_iter().collect();
        assert_eq!(majority_committed_index(&[1, 2, 3], &acks), 20);
    }

    #[test]
    fn majority_quorum_absent_member_is_zero() {
        // voters=[1,2,3]; 3 absent → treated as 0.
        // sorted desc: [30,20,0] → quorum=2 → index[1] = 20.
        let acks: HashMap<u64, u64> = [(1, 30), (2, 20)].into_iter().collect();
        assert_eq!(majority_committed_index(&[1, 2, 3], &acks), 20);
    }
}

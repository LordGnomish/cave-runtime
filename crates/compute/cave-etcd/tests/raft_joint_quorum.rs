// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD tests for raft_joint_quorum — joint committed-index calculation.
//!
//! Mirrors the behavioural contract of etcd v3.6:
//!   raft/quorum/joint.go   — JointConfig.CommittedIndex
//!   raft/quorum/majority.go — MajorityConfig.CommittedIndex
//!
//! In joint consensus a log entry is considered committed only when a
//! *strict majority* of BOTH the outgoing (Cold) AND the incoming (Cnew)
//! voter sets have acknowledged it. The committed index is therefore the
//! **minimum** of the per-config committed indices.

use cave_etcd::raft_joint_quorum::{joint_committed_index, majority_committed_index};
use std::collections::HashMap;

// ── majority_committed_index ──────────────────────────────────────────────────

/// Single voter → committed index equals its own match index.
#[test]
fn majority_single_voter_returns_its_match_index() {
    let mut acks: HashMap<u64, u64> = HashMap::new();
    acks.insert(1, 42);
    // voters = [1]
    assert_eq!(majority_committed_index(&[1], &acks), 42);
}

/// 3-voter cluster: majority (2 of 3) must have acknowledged.
/// acks: {1→10, 2→8, 3→5} → sorted desc: [10, 8, 5] → index [1] = 8 (2nd element = majority threshold).
#[test]
fn majority_three_voters_picks_second_highest() {
    let acks: HashMap<u64, u64> = [(1, 10), (2, 8), (3, 5)].into_iter().collect();
    assert_eq!(majority_committed_index(&[1, 2, 3], &acks), 8);
}

/// 5-voter cluster: majority = 3.
/// acks: {1→20, 2→15, 3→10, 4→5, 5→1} → sorted desc: [20,15,10,5,1] → index [2] = 10.
#[test]
fn majority_five_voters_picks_third_highest() {
    let acks: HashMap<u64, u64> = [(1, 20), (2, 15), (3, 10), (4, 5), (5, 1)]
        .into_iter()
        .collect();
    assert_eq!(majority_committed_index(&[1, 2, 3, 4, 5], &acks), 10);
}

/// Missing ack is treated as 0 (the member has not acknowledged anything).
#[test]
fn majority_missing_ack_treated_as_zero() {
    // voters = [1, 2, 3]; only 1 and 2 have acked; 3 missing → treated as 0.
    // sorted desc: [10, 5, 0] → index [1] = 5.
    let acks: HashMap<u64, u64> = [(1, 10), (2, 5)].into_iter().collect();
    assert_eq!(majority_committed_index(&[1, 2, 3], &acks), 5);
}

/// Empty voter set → committed index is u64::MAX (vacuously true: no voter can
/// prevent commitment), matching etcd's `MajorityConfig.CommittedIndex` for
/// empty configs.
#[test]
fn majority_empty_voters_returns_max() {
    let acks: HashMap<u64, u64> = HashMap::new();
    assert_eq!(majority_committed_index(&[], &acks), u64::MAX);
}

/// 2-voter cluster: majority = 2, both must have acked.
/// acks: {1→9, 2→7} → sorted desc: [9, 7] → index [1] = 7.
#[test]
fn majority_two_voters_requires_both() {
    let acks: HashMap<u64, u64> = [(1, 9), (2, 7)].into_iter().collect();
    assert_eq!(majority_committed_index(&[1, 2], &acks), 7);
}

// ── joint_committed_index ─────────────────────────────────────────────────────

/// No joint config (empty outgoing) → delegates to incoming only.
#[test]
fn joint_no_outgoing_delegates_to_incoming() {
    let acks: HashMap<u64, u64> = [(1, 10), (2, 8), (3, 5)].into_iter().collect();
    // outgoing = [] → vacuously u64::MAX; incoming majority = 8 → min = 8.
    let result = joint_committed_index(&[], &[1, 2, 3], &acks);
    assert_eq!(result, 8);
}

/// Both configs active → result is the min of both committed indices.
/// outgoing=[1,2,3] acks→[10,8,5] majority=8;
/// incoming=[1,2,4]  acks→[10,8,0] majority=8 (4 missing→0, picks 8).
#[test]
fn joint_takes_min_of_both_configs() {
    let acks: HashMap<u64, u64> = [(1, 10), (2, 8), (3, 5)].into_iter().collect();
    // outgoing=[1,2,3]: sorted desc [10,8,5] → idx 1 = 8
    // incoming=[1,2,4]: sorted desc [10,8,0] → idx 1 = 8
    // min(8, 8) = 8
    let result = joint_committed_index(&[1, 2, 3], &[1, 2, 4], &acks);
    assert_eq!(result, 8);
}

/// incoming is ahead of outgoing — the laggard wins.
/// outgoing=[1,2,3] low acks; incoming=[1,2] high acks → outgoing throttles.
#[test]
fn joint_slower_config_throttles_commit() {
    // outgoing=[1,2,3]: acks {1→10, 2→10, 3→2} → sorted desc [10,10,2] → idx 1 = 10.
    // incoming=[1,4]:    acks {1→10, 4 missing=0} → sorted desc [10,0] → idx 1 = 0.
    // min(10, 0) = 0.
    let acks: HashMap<u64, u64> = [(1, 10), (2, 10), (3, 2)].into_iter().collect();
    let result = joint_committed_index(&[1, 2, 3], &[1, 4], &acks);
    assert_eq!(result, 0);
}

/// Single-node cluster transitions to 3-node: outgoing=[1] committed at 50,
/// incoming=[1,2,3] only partially caught up → min dominates.
#[test]
fn joint_single_to_three_node_scenario() {
    // outgoing=[1]: ack {1→50} → majority = 50.
    // incoming=[1,2,3]: acks {1→50, 2→30, 3→10} → sorted [50,30,10] → idx 1 = 30.
    // min(50, 30) = 30.
    let acks: HashMap<u64, u64> = [(1, 50), (2, 30), (3, 10)].into_iter().collect();
    let result = joint_committed_index(&[1], &[1, 2, 3], &acks);
    assert_eq!(result, 30);
}

/// Both outgoing and incoming empty → u64::MAX (vacuously committed — used
/// during pre-bootstrap when there are no voters at all).
#[test]
fn joint_both_empty_returns_max() {
    let acks: HashMap<u64, u64> = HashMap::new();
    let result = joint_committed_index(&[], &[], &acks);
    assert_eq!(result, u64::MAX);
}

/// Overlapping voter sets where the intersection has a high ack but
/// non-overlap members lag — min of majorities governs.
#[test]
fn joint_overlap_intersection_scenario() {
    // outgoing=[1,2,3,4,5] acks→ majority (3rd highest of 5):
    //   {1→100, 2→90, 3→80, 4→70, 5→60} → sorted [100,90,80,70,60] → idx 2 = 80.
    // incoming=[3,4,5,6,7] acks:
    //   {3→80, 4→70, 5→60, 6 missing=0, 7 missing=0} → sorted [80,70,60,0,0] → idx 2 = 60.
    // min(80, 60) = 60.
    let acks: HashMap<u64, u64> = [(1, 100), (2, 90), (3, 80), (4, 70), (5, 60)]
        .into_iter()
        .collect();
    let result = joint_committed_index(&[1, 2, 3, 4, 5], &[3, 4, 5, 6, 7], &acks);
    assert_eq!(result, 60);
}

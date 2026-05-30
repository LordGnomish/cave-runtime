// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Portable-coverage unit tests for cave-ha's Raft engine internals.
//!
//! cave-ha is a fresh-implementation Raft consensus engine mapped to upstream
//! etcd (which vendors `go.etcd.io/raft`) at tag `v3.5.13`
//! (<https://github.com/etcd-io/etcd/tree/v3.5.13>). These tests target the pure,
//! deterministic public functions that etcd unit-tests directly in
//! `raft/log_test.go`, `raft/storage_test.go`, `raft/raft_snap_test.go`,
//! `raft/confchange/*`, and the quorum/read-only paths — the cave-side analogues
//! of `TestSlice`, `TestTerm`, `TestAppend`, `TestCompaction`, `TestStableTo`,
//! `TestLogRestore`, `TestSnapshotSucceed`/`TestSnapshotAbort`,
//! `TestConfState_Equivalent`, `TestClusterValidateConfigurationChange`,
//! `TestConfChangeV2*`, and `TestReadIndex`.
//!
//! Each assertion checks a concrete value derived from the implementation in
//! `crates/compute/cave-ha/src/raft/{log,snapshot,membership,read_only}.rs`.

use std::collections::BTreeSet;

use cave_ha::raft::log::{LogEntry, MemLog};
use cave_ha::raft::membership::{joint_for_remove, leave_joint, validate};
use cave_ha::raft::read_only::{LeaderLease, ReadMode, ReadOnlyQueue};
use cave_ha::raft::snapshot::{Snapshot, SnapshotReceiver};
use cave_ha::raft::types::{MembershipConfig, NodeId};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Build a fresh log with normal entries at indices 1..=n, all term `term`.
fn log_with(n: u64, term: u64) -> MemLog {
    let mut log = MemLog::new();
    let entries: Vec<LogEntry> = (1..=n)
        .map(|i| LogEntry::new_normal(i, term, vec![i as u8]))
        .collect();
    log.append(entries);
    log
}

fn set(ids: &[NodeId]) -> BTreeSet<NodeId> {
    ids.iter().copied().collect()
}

// ---------------------------------------------------------------------------
// MemLog::slice / term  (TestSlice, TestTerm, TestStorageEntries/Term)
// ---------------------------------------------------------------------------

#[test]
fn slice_returns_half_open_range_and_errors_below_snapshot() {
    // Indices 1..=5, terms all 7.
    let mut log = log_with(5, 7);

    // slice(2,4) is [2,4) -> indices 2,3.
    let got = log.slice(2, 4).expect("slice in range");
    let idxs: Vec<u64> = got.iter().map(|e| e.index).collect();
    assert_eq!(idxs, vec![2, 3], "slice(2,4) must be half-open [2,4)");

    // hi past the end clamps to last_index inclusive (no panic, no extra entries).
    let tail = log.slice(4, 99).expect("slice clamps hi");
    let tail_idxs: Vec<u64> = tail.iter().map(|e| e.index).collect();
    assert_eq!(tail_idxs, vec![4, 5], "hi beyond end clamps to entries.len()");

    // After compaction past index 3, slice starting at or below the snapshot errors.
    log.compact(3, 7);
    let err = log.slice(3, 5);
    assert!(
        err.is_err(),
        "slice(lo<=snapshot_index) must error LogCompacted, got {err:?}"
    );
    // Above the snapshot still works: first_index is now 4.
    let above = log.slice(4, 6).expect("slice above snapshot");
    let above_idxs: Vec<u64> = above.iter().map(|e| e.index).collect();
    assert_eq!(above_idxs, vec![4, 5]);
}

#[test]
fn term_resolves_snapshot_boundary_and_errors_when_compacted() {
    let mut log = log_with(5, 4);
    // term at a live index is that entry's term.
    assert_eq!(log.term(2).expect("live term"), 4);

    // Compact through index 3 at snapshot_term 9.
    log.compact(3, 9);
    // term(snapshot_index) == snapshot_term (special-cased before entry lookup).
    assert_eq!(
        log.term(3).expect("term at snapshot boundary"),
        9,
        "term(snapshot_index) returns snapshot_term"
    );
    // A live index above the snapshot keeps its original term.
    assert_eq!(log.term(4).expect("live term above snapshot"), 4);
    // An index strictly below the snapshot is compacted -> error.
    assert!(
        log.term(2).is_err(),
        "term of a compacted index must error"
    );
}

// ---------------------------------------------------------------------------
// MemLog::append  (TestAppend, TestFindConflict)
// ---------------------------------------------------------------------------

#[test]
fn append_truncates_conflicting_suffix_then_extends() {
    // Start with 1..=3 at term 1.
    let mut log = log_with(3, 1);
    assert_eq!(log.last_index(), 3);
    assert_eq!(log.len(), 3);

    // Re-append at index 2 with a different term: conflict truncates from offset(2)=1,
    // dropping old idx 2 and 3, then pushes the new idx 2 and 4.
    let conflicting = vec![
        LogEntry::new_normal(2, 5, vec![0xAA]),
        LogEntry::new_normal(3, 5, vec![0xBB]),
        LogEntry::new_normal(4, 5, vec![0xCC]),
    ];
    log.append(conflicting);

    assert_eq!(log.last_index(), 4, "log re-extends to the new suffix");
    assert_eq!(log.len(), 4);
    // index 1 (before the conflict) keeps its original term.
    assert_eq!(log.term(1).expect("idx1"), 1);
    // index 2 onward now carry the new term.
    assert_eq!(log.term(2).expect("idx2"), 5);
    assert_eq!(log.term(4).expect("idx4"), 5);
    // The new data at index 2 replaced the old.
    assert_eq!(log.entry(2).expect("idx2 entry").data, vec![0xAA]);
}

// ---------------------------------------------------------------------------
// MemLog::compact  (TestCompaction, TestStorageCompact)
// ---------------------------------------------------------------------------

#[test]
fn compact_discards_through_index_and_sets_snapshot_meta() {
    let mut log = log_with(5, 2);

    log.compact(3, 2);

    assert_eq!(log.snapshot_index(), 3, "snapshot_index advances to compact idx");
    assert_eq!(log.snapshot_term(), 2, "snapshot_term records the term");
    assert_eq!(log.first_index(), 4, "first_index == snapshot_index + 1");
    assert_eq!(log.last_index(), 5, "tail entries survive compaction");
    assert_eq!(log.len(), 2, "only indices 4,5 remain above the snapshot");

    // A pre-snapshot index can no longer be fetched.
    assert!(log.entry(2).is_err(), "compacted entry is gone");
    // A no-op compact at/under the current snapshot leaves state unchanged.
    log.compact(1, 99);
    assert_eq!(log.snapshot_index(), 3);
    assert_eq!(log.snapshot_term(), 2);
}

// ---------------------------------------------------------------------------
// MemLog::truncate_to  (TestStableTo, TestUnstableTruncateAndAppend)
// ---------------------------------------------------------------------------

#[test]
fn truncate_to_rolls_back_tail() {
    let mut log = log_with(5, 1);
    log.truncate_to(3);
    assert_eq!(log.last_index(), 3, "truncate_to keeps through index 3");
    assert_eq!(log.len(), 3);
    // Truncating below first_index clears the in-memory tail entirely.
    log.truncate_to(0);
    assert!(log.is_empty(), "truncate below first_index empties the log");
    assert_eq!(log.len(), 0);
}

// ---------------------------------------------------------------------------
// MemLog::last_membership  (TestLogRestore / confchange restore)
// ---------------------------------------------------------------------------

#[test]
fn last_membership_returns_most_recent_change_entry() {
    let mut log = MemLog::new();
    let cfg_a = MembershipConfig::single(1);
    let cfg_b = MembershipConfig {
        voters: set(&[1, 2, 3]),
        ..Default::default()
    };

    log.append(vec![
        LogEntry::new_normal(1, 1, vec![]),
        LogEntry::new_membership(2, 1, &cfg_a).expect("encode A"),
        LogEntry::new_normal(3, 1, vec![]),
        LogEntry::new_membership(4, 1, &cfg_b).expect("encode B"),
    ]);

    // Scans backward -> the later config (cfg_b at index 4) wins.
    let found = log.last_membership().expect("a membership entry exists");
    assert_eq!(found, cfg_b, "most recent MembershipChange entry is returned");
    assert_eq!(found.voters, set(&[1, 2, 3]));

    // A log with no membership entry returns None.
    let plain = log_with(2, 1);
    assert!(plain.last_membership().is_none());
}

// ---------------------------------------------------------------------------
// Snapshot::chunks  (TestSnapshotSucceed chunking path)
// ---------------------------------------------------------------------------

#[test]
fn chunks_split_contiguously_and_terminate_with_done() {
    let snap = Snapshot::new(10, 3, MembershipConfig::single(1), vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

    // 10 bytes / chunk_size 4 -> [0..4), [4..8), [8..10) = 3 chunks.
    let chunks = snap.chunks(4);
    assert_eq!(chunks.len(), 3, "ceil(10/4) == 3 chunks");
    assert_eq!(chunks[0].offset, 0);
    assert_eq!(chunks[1].offset, 4, "offsets are contiguous by data length");
    assert_eq!(chunks[2].offset, 8);
    assert_eq!(chunks[0].data, vec![0, 1, 2, 3]);
    assert_eq!(chunks[2].data, vec![8, 9], "final chunk holds the remainder");
    assert!(!chunks[0].done);
    assert!(!chunks[1].done);
    assert!(chunks[2].done, "only the last chunk is done");

    // Empty data yields exactly one already-done chunk.
    let empty = Snapshot::new(1, 1, MembershipConfig::single(1), vec![]);
    let ec = empty.chunks(4);
    assert_eq!(ec.len(), 1);
    assert!(ec[0].done);
    assert!(ec[0].data.is_empty());
}

// ---------------------------------------------------------------------------
// SnapshotReceiver::feed  (TestSnapshotSucceed / TestSnapshotAbort)
// ---------------------------------------------------------------------------

#[test]
fn receiver_assembles_in_order_chunks_and_resets_on_gap() {
    let snap = Snapshot::new(7, 2, MembershipConfig::single(1), vec![10, 20, 30, 40, 50]);
    let chunks = snap.chunks(2); // [10,20] [30,40] [50]

    // In-order feed: only the final (done) chunk yields Some(Snapshot).
    let mut rx = SnapshotReceiver::new();
    assert!(rx.feed(chunks[0].clone()).is_none());
    assert!(rx.feed(chunks[1].clone()).is_none());
    let done = rx.feed(chunks[2].clone()).expect("done chunk completes snapshot");
    assert_eq!(done.data, vec![10, 20, 30, 40, 50], "reassembled bytes match");
    assert_eq!(done.meta.index, 7);
    assert_eq!(done.meta.term, 2);

    // Out-of-order: after the offset-0 chunk, skipping to the last chunk
    // (offset != next_offset) returns None and resets the buffer.
    let mut rx2 = SnapshotReceiver::new();
    assert!(rx2.feed(chunks[0].clone()).is_none());
    assert!(
        rx2.feed(chunks[2].clone()).is_none(),
        "a gapped offset returns None"
    );
    assert!(rx2.buffer.is_empty(), "buffer is reset after an out-of-order chunk");
    assert_eq!(rx2.next_offset, 0, "next_offset rewinds to 0 on reset");
}

// ---------------------------------------------------------------------------
// membership::joint_for_remove / leave_joint  (TestConfChangeV2*)
// ---------------------------------------------------------------------------

#[test]
fn joint_for_remove_then_leave_joint_round_trip() {
    let current = MembershipConfig {
        voters: set(&[1, 2, 3]),
        ..Default::default()
    };

    // Phase 1: enter joint config removing node 3.
    let joint = joint_for_remove(&current, 3);
    assert!(joint.is_joint(), "removal produces a joint (C_old,new) config");
    assert_eq!(joint.voters, set(&[1, 2]), "node 3 removed from incoming voters");
    assert!(!joint.voters.contains(&3));
    assert_eq!(
        joint.voters_outgoing.as_ref().expect("outgoing set present"),
        &set(&[1, 2, 3]),
        "voters_outgoing preserves the old voter set"
    );
    assert!(joint.auto_leave, "joint config is marked auto_leave");

    // Phase 2: leave joint -> drop outgoing set, clear auto_leave, keep C_new voters.
    let final_cfg = leave_joint(&joint);
    assert!(!final_cfg.is_joint(), "leave_joint clears the joint marker");
    assert!(final_cfg.voters_outgoing.is_none());
    assert!(!final_cfg.auto_leave);
    assert_eq!(final_cfg.voters, set(&[1, 2]), "C_new voters are preserved");
}

// ---------------------------------------------------------------------------
// membership::validate  (TestClusterValidateConfigurationChange)
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_overlap_and_multi_removal_but_accepts_single_change() {
    let current = MembershipConfig {
        voters: set(&[1, 2, 3]),
        ..Default::default()
    };

    // Valid single-voter removal (3 -> {1,2}) is accepted.
    let ok = MembershipConfig {
        voters: set(&[1, 2]),
        ..Default::default()
    };
    assert!(validate(&current, &ok).is_ok(), "single voter removal is valid");

    // Empty voter set is rejected.
    let empty = MembershipConfig::default();
    assert!(validate(&current, &empty).is_err(), "no voters is invalid");

    // A node appearing in both voters and learners is rejected.
    let overlap = MembershipConfig {
        voters: set(&[1, 2]),
        learners: set(&[2]),
        ..Default::default()
    };
    assert!(
        validate(&current, &overlap).is_err(),
        "voter/learner overlap is invalid"
    );

    // Removing more than one voter at once is rejected.
    let multi = MembershipConfig {
        voters: set(&[1]),
        ..Default::default()
    };
    assert!(
        validate(&current, &multi).is_err(),
        "removing >1 voter at a time is invalid"
    );
}

// ---------------------------------------------------------------------------
// MembershipConfig::is_joint / all_voters / all_nodes  (TestConfState_Equivalent)
// ---------------------------------------------------------------------------

#[test]
fn joint_config_unions_old_and_new_voters_and_includes_learners() {
    let joint = MembershipConfig {
        voters: set(&[3, 4, 5]),
        learners: set(&[9]),
        voters_outgoing: Some(set(&[1, 2, 3])),
        auto_leave: true,
    };

    assert!(joint.is_joint(), "voters_outgoing == Some marks a joint config");
    // all_voters == new ∪ old.
    assert_eq!(
        joint.all_voters(),
        set(&[1, 2, 3, 4, 5]),
        "all_voters is the union of incoming and outgoing voter sets"
    );
    // all_nodes == voters ∪ learners ∪ outgoing.
    assert_eq!(
        joint.all_nodes(),
        set(&[1, 2, 3, 4, 5, 9]),
        "all_nodes adds learners to the voter union"
    );

    // A non-joint single config: all_voters == voters, no outgoing widening.
    let single = MembershipConfig::single(7);
    assert!(!single.is_joint());
    assert_eq!(single.all_voters(), set(&[7]));
    assert_eq!(single.all_nodes(), set(&[7]));
}

// ---------------------------------------------------------------------------
// ReadOnlyQueue::add + ack  (TestReadIndex)
// ---------------------------------------------------------------------------

#[test]
fn read_only_queue_drains_request_exactly_at_quorum() {
    use tokio::sync::oneshot;

    let mut q = ReadOnlyQueue::new(ReadMode::ReadIndex);
    assert_eq!(q.mode(), ReadMode::ReadIndex);
    assert!(q.is_empty());

    // Add a read pinned at commit index 42; ids start at 1.
    let (tx, _rx) = oneshot::channel();
    let id = q.add(42, tx);
    assert_eq!(id, 1, "request ids start at 1");
    assert!(!q.is_empty(), "queue holds the pending read");

    // quorum == 2: a single distinct peer ack is below quorum -> nothing drains.
    let drained = q.ack(10, 2);
    assert!(drained.is_empty(), "1 ack < quorum 2: not ready");
    assert!(!q.is_empty(), "request still pending");

    // Re-acking the same peer does not grow the ack set (HashSet semantics).
    let still = q.ack(10, 2);
    assert!(still.is_empty(), "duplicate peer ack does not reach quorum");

    // A second distinct peer reaches quorum -> the request drains out.
    let ready = q.ack(11, 2);
    assert_eq!(ready.len(), 1, "2 distinct acks >= quorum 2: drains");
    assert_eq!(ready[0].index, 42, "drained request carries its commit index");
    assert_eq!(ready[0].acks.len(), 2, "drained request has both peer acks");
    assert!(q.is_empty(), "request removed from the queue after draining");
}

// ---------------------------------------------------------------------------
// LeaderLease::renew / is_valid / invalidate  (lease-read tests)
// ---------------------------------------------------------------------------

#[test]
fn leader_lease_validity_transitions() {
    use std::time::Duration;

    // A fresh lease has never been granted -> invalid.
    let mut lease = LeaderLease::new(Duration::from_secs(60));
    assert!(!lease.is_valid(), "ungranted lease is invalid");

    // Renew grants the lease; within the (long) window it is valid.
    lease.renew();
    assert!(lease.is_valid(), "renewed lease is valid within its window");

    // Invalidate drops the grant -> invalid again.
    lease.invalidate();
    assert!(!lease.is_valid(), "invalidated lease is no longer valid");

    // A zero-duration lease is never valid even immediately after renew
    // (elapsed() >= 0 is never strictly < 0).
    let mut expired = LeaderLease::new(Duration::from_secs(0));
    expired.renew();
    assert!(!expired.is_valid(), "zero-duration lease cannot be valid");
}

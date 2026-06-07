// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Raft consensus integration test — a 3-node HA cluster driven entirely
//! through the public `RaftBackend` API.
//!
//! Mirrors `openbao/physical/raft` HA storage: a leader is elected
//! (RequestVote), proposes log entries, replicates them with AppendEntries
//! (§5.3 consistency check + conflict truncation), the quorum-acked index is
//! committed, and every replica applies the same entries and converges on the
//! same key/value state. Also exercises the safety properties: an
//! inconsistent AppendEntries is rejected, conflicting tails are overwritten,
//! and an entry not present on a majority is never committed.

use cave_vault::storage::raft::{LogEntry, LogOp, RaftBackend};
use cave_vault::storage::Backend;

/// The highest log index replicated on a majority of the cluster — the index
/// a leader may advance `commitIndex` to (Raft §5.3/§5.4). Sorting ascending,
/// the majority-acked index is the element at position `(n-1)/2` (the value
/// such that `ceil((n+1)/2)` nodes hold an index ≥ it).
fn quorum_commit_index(mut match_indices: Vec<u64>) -> u64 {
    match_indices.sort_unstable();
    let n = match_indices.len();
    match_indices[(n - 1) / 2]
}

/// Replicate the leader's full log to a follower via one AppendEntries, then
/// have the follower learn the leader commit. Returns AppendEntries success.
fn replicate_all(leader: &RaftBackend, follower: &RaftBackend, leader_commit: u64) -> bool {
    let entries = leader.log_entries_from(0);
    follower
        .append_entries(0, 0, entries, leader_commit)
        .expect("append_entries")
}

#[test]
fn elects_leader_then_replicates_and_commits_across_quorum() {
    let leader = RaftBackend::new();
    let f1 = RaftBackend::new();
    let f2 = RaftBackend::new();

    // ── Election (term 1): candidate `leader` (id 0) wins ───────────────────
    leader.bump_term(1);
    assert!(leader.cast_vote(0, 1)); // votes for itself
    assert!(f1.cast_vote(0, 1)); // follower grants
    assert!(f2.cast_vote(0, 1)); // follower grants
    f1.bump_term(1);
    f2.bump_term(1);

    // ── Leader proposes three writes ───────────────────────────────────────
    leader.propose(LogOp::Put { path: "kv/a".into(), value: b"1".to_vec() });
    leader.propose(LogOp::Put { path: "kv/b".into(), value: b"2".to_vec() });
    leader.propose(LogOp::Put { path: "kv/c".into(), value: b"3".to_vec() });
    assert_eq!(leader.last_log_index(), 3);

    // ── Replicate to both followers (AppendEntries) ────────────────────────
    assert!(replicate_all(&leader, &f1, 0));
    assert!(replicate_all(&leader, &f2, 0));
    assert_eq!(f1.last_log_index(), 3);
    assert_eq!(f2.last_log_index(), 3);

    // ── Commit at the quorum-acked index ───────────────────────────────────
    let match_indices = vec![
        leader.last_log_index(),
        f1.last_log_index(),
        f2.last_log_index(),
    ];
    let commit = quorum_commit_index(match_indices);
    assert_eq!(commit, 3, "all three have index 3 → commit 3");

    leader.mark_committed(commit).unwrap();
    leader.apply_committed().unwrap();
    // Followers learn the commit on the next AppendEntries heartbeat.
    assert!(replicate_all(&leader, &f1, commit));
    assert!(replicate_all(&leader, &f2, commit));
    f1.apply_committed().unwrap();
    f2.apply_committed().unwrap();

    // ── Convergence: all three replicas agree ──────────────────────────────
    for node in [&leader, &f1, &f2] {
        assert_eq!(node.get("kv/a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(node.get("kv/b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(node.get("kv/c").unwrap(), Some(b"3".to_vec()));
        assert_eq!(node.commit_index(), 3);
        assert_eq!(node.last_applied(), 3);
    }
}

#[test]
fn append_entries_rejects_log_gap() {
    let f = RaftBackend::new();
    f.bump_term(1);
    // Leader claims a prev_log_index of 5 the follower has never seen.
    let entry = LogEntry {
        index: 6,
        term: 1,
        op: LogOp::Put { path: "kv/x".into(), value: b"v".to_vec() },
    };
    let ok = f.append_entries(5, 1, vec![entry], 0).unwrap();
    assert!(!ok, "consistency check must reject a gap");
    assert_eq!(f.last_log_index(), 0, "log must be untouched");
}

#[test]
fn append_entries_overwrites_conflicting_tail() {
    let f = RaftBackend::new();
    f.bump_term(1);
    // Follower has three term-1 entries.
    f.propose(LogOp::Put { path: "k/1".into(), value: b"a".to_vec() });
    f.propose(LogOp::Put { path: "k/2".into(), value: b"b".to_vec() });
    f.propose(LogOp::Put { path: "k/3".into(), value: b"c".to_vec() });

    // New leader (term 2) overwrites from index 2 with a different entry.
    f.bump_term(2);
    let new_entry = LogEntry {
        index: 2,
        term: 2,
        op: LogOp::Put { path: "k/2new".into(), value: b"z".to_vec() },
    };
    let ok = f.append_entries(1, 1, vec![new_entry], 0).unwrap();
    assert!(ok);
    assert_eq!(f.last_log_index(), 2, "conflicting tail (idx 3) dropped");
    let tail = f.log_entries_from(1);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].term, 2);
    assert_eq!(tail[0].op.name(), "put");
}

#[test]
fn idempotent_append_does_not_duplicate() {
    let leader = RaftBackend::new();
    let f = RaftBackend::new();
    leader.bump_term(1);
    f.bump_term(1);
    leader.propose(LogOp::Put { path: "kv/a".into(), value: b"1".to_vec() });

    // Same AppendEntries delivered twice (retransmit) must be a no-op.
    assert!(replicate_all(&leader, &f, 0));
    assert!(replicate_all(&leader, &f, 0));
    assert_eq!(f.last_log_index(), 1);
}

#[test]
fn entry_not_on_majority_is_not_committed() {
    let leader = RaftBackend::new();
    let f1 = RaftBackend::new();
    let f2 = RaftBackend::new();
    for n in [&leader, &f1, &f2] {
        n.bump_term(1);
    }

    // idx 1 replicated to f1 only; idx 2 replicated to nobody.
    leader.propose(LogOp::Put { path: "kv/safe".into(), value: b"1".to_vec() });
    let e1 = leader.log_entries_from(0);
    assert!(f1.append_entries(0, 0, e1, 0).unwrap());
    leader.propose(LogOp::Put { path: "kv/risky".into(), value: b"2".to_vec() });

    // matchIndex: leader=2, f1=1, f2=0  → majority-acked index = 1.
    let commit = quorum_commit_index(vec![
        leader.last_log_index(),
        f1.last_log_index(),
        f2.last_log_index(),
    ]);
    assert_eq!(commit, 1, "only idx 1 is on a majority");

    leader.mark_committed(commit).unwrap();
    leader.apply_committed().unwrap();
    assert_eq!(leader.get("kv/safe").unwrap(), Some(b"1".to_vec()));
    // The risky (uncommitted, minority) entry must NOT be applied.
    assert!(leader.get("kv/risky").unwrap().is_none());
    assert_eq!(leader.last_applied(), 1);
}

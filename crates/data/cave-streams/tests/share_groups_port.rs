// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral parity tests for KIP-932 "Queues for Kafka" share groups.
//!
//! Ported faithfully from Apache Kafka 4.2.0:
//!   - `server/.../share/fetch/RecordState.java`          (Available=0,Acquired=1,Acknowledged=2,Archived=4)
//!   - `clients/.../consumer/AcknowledgeType.java`        (Accept=1,Release=2,Reject=3,Renew=4)
//!   - `core/.../share/SharePartition.java`               (acquire/acknowledge/release/lock-timeout)
//!   - `core/.../share/SharePartitionManager.java`
//!   - `group-coordinator/.../modern/share/ShareGroup.java`
//!   - `server/.../share/session/ShareSession.java`
//!   - `share-coordinator/.../ShareGroupOffset.java` + `server-common/.../PersisterStateBatch.java`
//!
//! These are RED until `src/kafka_share_groups.rs` lands.

use cave_streams::kafka_share_groups::{
    AcknowledgeType, MemberState, RecordState, ShareError, ShareGroup, ShareGroupConfig,
    ShareGroupOffset, ShareSession, SharePartition, SharePartitionKey, SharePartitionManager,
};

// ── RecordState ──────────────────────────────────────────────────────────────

#[test]
fn record_state_ids_round_trip() {
    for st in [
        RecordState::Available,
        RecordState::Acquired,
        RecordState::Acknowledged,
        RecordState::Archived,
    ] {
        assert_eq!(RecordState::from_id(st.id()).unwrap(), st);
    }
    assert_eq!(RecordState::from_id(0).unwrap(), RecordState::Available);
    assert_eq!(RecordState::from_id(1).unwrap(), RecordState::Acquired);
    assert_eq!(RecordState::from_id(2).unwrap(), RecordState::Acknowledged);
    assert_eq!(RecordState::from_id(4).unwrap(), RecordState::Archived);
}

#[test]
fn record_state_id_3_is_skipped() {
    // Upstream intentionally skips id 3 — guard against renumbering.
    assert_eq!(RecordState::Archived.id(), 4);
    assert!(RecordState::from_id(3).is_err());
    assert!(RecordState::from_id(7).is_err());
}

#[test]
fn record_state_terminals_reject_all_transitions() {
    for &term in &[RecordState::Acknowledged, RecordState::Archived] {
        for &to in &[
            RecordState::Available,
            RecordState::Acquired,
            RecordState::Acknowledged,
            RecordState::Archived,
        ] {
            assert!(
                term.validate_transition(to).is_err(),
                "{:?}->{:?} must be rejected (terminal)",
                term,
                to
            );
        }
    }
}

#[test]
fn record_state_available_only_to_acquired() {
    assert!(RecordState::Available
        .validate_transition(RecordState::Acquired)
        .is_ok());
    assert!(RecordState::Available
        .validate_transition(RecordState::Available)
        .is_err());
    assert!(RecordState::Available
        .validate_transition(RecordState::Acknowledged)
        .is_err());
    assert!(RecordState::Available
        .validate_transition(RecordState::Archived)
        .is_err());
}

#[test]
fn record_state_acquired_transitions() {
    assert!(RecordState::Acquired
        .validate_transition(RecordState::Available)
        .is_ok());
    assert!(RecordState::Acquired
        .validate_transition(RecordState::Acknowledged)
        .is_ok());
    assert!(RecordState::Acquired
        .validate_transition(RecordState::Archived)
        .is_ok());
    // same-state is rejected even for non-terminal Acquired
    assert!(RecordState::Acquired
        .validate_transition(RecordState::Acquired)
        .is_err());
}

#[test]
fn acknowledge_type_ids_round_trip() {
    assert_eq!(AcknowledgeType::Accept.id(), 1);
    assert_eq!(AcknowledgeType::Release.id(), 2);
    assert_eq!(AcknowledgeType::Reject.id(), 3);
    assert_eq!(AcknowledgeType::Renew.id(), 4);
    for ack in [
        AcknowledgeType::Accept,
        AcknowledgeType::Release,
        AcknowledgeType::Reject,
        AcknowledgeType::Renew,
    ] {
        assert_eq!(AcknowledgeType::from_id(ack.id()).unwrap(), ack);
    }
    assert!(AcknowledgeType::from_id(0).is_err());
    assert!(AcknowledgeType::from_id(5).is_err());
}

// ── SharePartition acquire/acknowledge ───────────────────────────────────────

fn fresh(start: i64) -> SharePartition {
    SharePartition::new("group-1", "topic", 0, start)
}

#[test]
fn acquire_fresh_allocates_single_batch() {
    let mut sp = fresh(0);
    let got = sp.acquire("m1", 10, 1000);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].first_offset, 0);
    assert_eq!(got[0].last_offset, 9);
    assert_eq!(got[0].delivery_count, 1);
    assert_eq!(sp.next_fetch_offset(), 10);
    assert_eq!(sp.batches_snapshot().len(), 1);
}

#[test]
fn acquire_zero_records_is_noop() {
    let mut sp = fresh(0);
    let got = sp.acquire("m1", 0, 1000);
    assert!(got.is_empty());
    assert_eq!(sp.next_fetch_offset(), 0);
    assert!(sp.batches_snapshot().is_empty());
}

#[test]
fn acknowledge_accept_marks_acknowledged_and_clears_lock() {
    let mut sp = fresh(0);
    sp.acquire("m1", 5, 1000);
    sp.acknowledge("m1", 0, 4, AcknowledgeType::Accept, 2000).unwrap();
    let b = &sp.batches_snapshot()[0];
    assert_eq!(b.batch_state.state, RecordState::Acknowledged);
    assert_eq!(b.batch_state.lock_expires_at_ms, None);
}

#[test]
fn acknowledge_release_returns_to_available_and_next_acquire_bumps_delivery() {
    let mut sp = fresh(0);
    sp.acquire("m1", 4, 1000);
    sp.acknowledge("m1", 0, 3, AcknowledgeType::Release, 2000).unwrap();
    let b = &sp.batches_snapshot()[0];
    assert_eq!(b.batch_state.state, RecordState::Available);
    assert_eq!(b.batch_state.lock_expires_at_ms, None);
    let got = sp.acquire("m2", 4, 3000);
    assert_eq!(got[0].delivery_count, 2);
}

#[test]
fn acknowledge_reject_archives() {
    let mut sp = fresh(0);
    sp.acquire("m1", 2, 1000);
    sp.acknowledge("m1", 0, 1, AcknowledgeType::Reject, 2000).unwrap();
    assert_eq!(sp.batches_snapshot()[0].batch_state.state, RecordState::Archived);
}

#[test]
fn acknowledge_renew_extends_lock_only() {
    let mut sp = fresh(0);
    sp.acquire("m1", 2, 1000);
    let before = sp.batches_snapshot()[0].batch_state.lock_expires_at_ms.unwrap();
    sp.acknowledge("m1", 0, 1, AcknowledgeType::Renew, 5000).unwrap();
    let b = &sp.batches_snapshot()[0];
    assert_eq!(b.batch_state.state, RecordState::Acquired);
    let after = b.batch_state.lock_expires_at_ms.unwrap();
    assert!(after > before, "renew must push deadline forward ({} > {})", after, before);
}

#[test]
fn acknowledge_wrong_member_rejected() {
    let mut sp = fresh(0);
    sp.acquire("m1", 2, 1000);
    let err = sp.acknowledge("m2", 0, 1, AcknowledgeType::Accept, 2000).unwrap_err();
    assert!(matches!(err, ShareError::NotAcquiredByMember { .. }));
}

#[test]
fn acknowledge_unknown_offset_rejected() {
    let mut sp = fresh(0);
    sp.acquire("m1", 2, 1000);
    let err = sp.acknowledge("m1", 99, 99, AcknowledgeType::Accept, 2000).unwrap_err();
    assert!(matches!(err, ShareError::OffsetNotFound(99)));
}

#[test]
fn acknowledge_already_terminal_rejected_as_invalid_state() {
    let mut sp = fresh(0);
    sp.acquire("m1", 2, 1000);
    sp.acknowledge("m1", 0, 1, AcknowledgeType::Accept, 2000).unwrap();
    let err = sp.acknowledge("m1", 0, 1, AcknowledgeType::Accept, 3000).unwrap_err();
    assert!(matches!(err, ShareError::InvalidRecordState { .. }));
}

// ── lock sweep ───────────────────────────────────────────────────────────────

#[test]
fn sweep_expired_releases_acquired_batch() {
    let mut sp = SharePartition::new("g", "t", 0, 0).with_record_lock_duration_ms(1000);
    sp.acquire("m1", 2, 0); // deadline = 1000
    let released = sp.sweep_expired_locks(5000);
    assert_eq!(released.len(), 1);
    assert_eq!(sp.batches_snapshot()[0].batch_state.state, RecordState::Available);
}

#[test]
fn sweep_keeps_unexpired_lock() {
    let mut sp = fresh(0); // default 30s lock
    sp.acquire("m1", 2, 100_000);
    assert!(sp.sweep_expired_locks(1000).is_empty());
}

#[test]
fn delivery_cap_archives_on_release() {
    // max_delivery_count = 3: three acquire+release cycles end Archived.
    let mut sp = SharePartition::new("g", "t", 0, 0).with_max_delivery_count(3);
    for now in [0u64, 100, 200] {
        sp.acquire("m1", 2, now);
        sp.acknowledge("m1", 0, 1, AcknowledgeType::Release, now + 1).unwrap();
    }
    assert_eq!(sp.batches_snapshot()[0].batch_state.state, RecordState::Archived);
}

#[test]
fn delivery_cap_one_release_archives_then_fresh_batch() {
    let mut sp = SharePartition::new("g", "t", 0, 0).with_max_delivery_count(1);
    sp.acquire("m1", 2, 1000);
    sp.acknowledge("m1", 0, 1, AcknowledgeType::Release, 2000).unwrap();
    assert_eq!(sp.batches_snapshot()[0].batch_state.state, RecordState::Archived);
    let got = sp.acquire("m1", 2, 3000);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].first_offset, 2, "capped batch is skipped; fresh batch at next_fetch");
}

// ── move_start_offset ────────────────────────────────────────────────────────

#[test]
fn move_start_offset_drops_fully_stale_and_bumps_epoch() {
    let mut sp = fresh(0);
    sp.acquire("m1", 10, 1000);
    sp.acknowledge("m1", 0, 9, AcknowledgeType::Accept, 2000).unwrap();
    let epoch_before = sp.state_epoch();
    sp.move_start_offset(10);
    assert_eq!(sp.state_epoch(), epoch_before + 1);
    assert!(sp.batches_snapshot().is_empty());
}

#[test]
fn move_start_offset_keeps_straddling_batch_whole() {
    let mut sp = fresh(10);
    sp.acquire("m1", 5, 1000); // [10,14]
    sp.move_start_offset(12);
    assert_eq!(sp.batches_snapshot().len(), 1, "partial-overlap batch kept (no split)");
}

#[test]
fn move_start_offset_noop_when_not_advancing() {
    let mut sp = fresh(10);
    let epoch_before = sp.state_epoch();
    sp.move_start_offset(5);
    assert_eq!(sp.state_epoch(), epoch_before, "no epoch bump when new_start <= start");
}

// ── snapshot / persister form ────────────────────────────────────────────────

#[test]
fn snapshot_emits_share_group_offset() {
    let mut sp = fresh(0);
    sp.acquire("m1", 2, 1000);
    let snap: ShareGroupOffset = sp.snapshot();
    assert_eq!(snap.group_id, "group-1");
    assert_eq!(snap.partition, 0);
    assert_eq!(snap.batches.len(), 1);
    assert_eq!(snap.batches[0].state, RecordState::Acquired);
    assert_eq!(snap.batches[0].delivery_count, 1);
}

// ── SharePartitionManager ────────────────────────────────────────────────────

#[test]
fn manager_isolates_distinct_keys() {
    let mgr = SharePartitionManager::default();
    let k0 = SharePartitionKey::new("g1", "t", 0);
    let k1 = SharePartitionKey::new("g1", "t", 1);
    mgr.get_or_create(k0.clone(), 0);
    mgr.get_or_create(k1.clone(), 100);
    assert_eq!(mgr.len(), 2);
    let snaps = mgr.snapshot();
    let s0 = snaps.iter().find(|s| s.partition == 0).unwrap();
    let s1 = snaps.iter().find(|s| s.partition == 1).unwrap();
    assert_eq!(s0.start_offset, 0);
    assert_eq!(s1.start_offset, 100);
}

#[test]
fn manager_tick_sweep_counts_flipped_batches() {
    let mgr = SharePartitionManager::default();
    let k = SharePartitionKey::new("g1", "t", 0);
    mgr.get_or_create(k.clone(), 0);
    mgr.with(&k, |sp| sp.acquire("m1", 2, 0)).unwrap(); // default 30s lock -> deadline 30000
    assert_eq!(mgr.tick_sweep(60_000), 1);
}

// ── ShareGroup membership + epochs ───────────────────────────────────────────

#[test]
fn share_group_epoch_moves_on_join_and_leave() {
    let mut g = ShareGroup::new("sg");
    assert_eq!(g.group_epoch(), 0);
    assert_eq!(g.join("m1"), 1);
    assert_eq!(g.join("m2"), 2);
    assert_eq!(g.leave("m1").unwrap(), 3);
    assert_eq!(g.member_count(), 1);
}

#[test]
fn share_group_member_stabilises() {
    let mut g = ShareGroup::new("sg");
    g.join("m1");
    assert_eq!(g.member_state("m1"), Some(MemberState::Joining));
    g.stabilise("m1").unwrap();
    assert_eq!(g.member_state("m1"), Some(MemberState::Stable));
}

#[test]
fn share_group_unknown_member_errors() {
    let mut g = ShareGroup::new("sg");
    assert!(matches!(g.stabilise("ghost"), Err(ShareError::UnknownGroupMember(_))));
    assert!(matches!(g.leave("ghost"), Err(ShareError::UnknownGroupMember(_))));
}

#[test]
fn share_group_session_epoch_is_monotonic_per_member() {
    let mut g = ShareGroup::new("sg");
    g.join("m1");
    assert_eq!(g.bump_session_epoch("m1").unwrap(), 1);
    assert_eq!(g.bump_session_epoch("m1").unwrap(), 2);
    assert_eq!(g.bump_session_epoch("m1").unwrap(), 3);
}

#[test]
fn share_group_config_kip932_defaults() {
    let c = ShareGroupConfig::default();
    assert_eq!(c.record_lock_duration_ms, 30_000);
    assert_eq!(c.delivery_count_limit, 5);
    assert_eq!(c.session_timeout_ms, 45_000);
}

// ── ShareSession ─────────────────────────────────────────────────────────────

#[test]
fn share_session_advance_is_epoch_validated() {
    let mut s = ShareSession::new("g", "m1", "conn-1");
    assert_eq!(s.advance(0).unwrap(), 1);
    assert_eq!(s.advance(1).unwrap(), 2);
    assert!(matches!(s.advance(99), Err(ShareError::SessionEpochMismatch { .. })));
}

#[test]
fn share_session_partition_set_dedups() {
    let mut s = ShareSession::new("g", "m1", "conn-1");
    let k = SharePartitionKey::new("g", "t", 0);
    s.add_partition(k.clone());
    s.add_partition(k.clone());
    assert_eq!(s.partition_count(), 1);
    assert!(s.remove_partition(&k));
    assert!(!s.remove_partition(&k));
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's replication subsystem:
//   * src/backend/replication/slot.c       — replication slots, the
//     restart_lsn WAL-retention floor, forward-only advance, name clashes
//   * src/backend/replication/walsender.c   — standby write/flush/apply
//     feedback and the byte lag it implies
//   * src/backend/replication/logical/reorderbuffer.c + pgoutput — logical
//     decoding: changes are buffered per xact and streamed only at commit,
//     in commit order; aborted xacts emit nothing.

use cave_rdbms::storage::replication::{
    Change, DecodedChange, ReorderBuffer, ReplicationSlots, SlotError, SlotKind, StandbyFeedback,
};

#[test]
fn slot_reserve_and_forward_only_advance() {
    let mut slots = ReplicationSlots::new();
    slots.create("s1", SlotKind::Physical, 100).unwrap();
    assert_eq!(slots.restart_lsn("s1"), Some(100));

    // duplicate name is rejected
    assert_eq!(
        slots.create("s1", SlotKind::Physical, 50),
        Err(SlotError::AlreadyExists)
    );

    // advance moves the floor forward
    slots.advance("s1", 200).unwrap();
    assert_eq!(slots.restart_lsn("s1"), Some(200));

    // advancing backwards is a no-op (pg_replication_slot_advance only moves up)
    slots.advance("s1", 150).unwrap();
    assert_eq!(slots.restart_lsn("s1"), Some(200));
}

#[test]
fn oldest_restart_lsn_pins_wal_retention() {
    let mut slots = ReplicationSlots::new();
    slots.create("phys", SlotKind::Physical, 200).unwrap();
    slots.create("logi", SlotKind::Logical, 300).unwrap();
    // WAL before the oldest slot's restart_lsn may be recycled.
    assert_eq!(slots.oldest_restart_lsn(), Some(200));

    slots.drop("phys").unwrap();
    assert_eq!(slots.oldest_restart_lsn(), Some(300));

    assert_eq!(slots.drop("missing"), Err(SlotError::NotFound));
    // logical slots track confirmed_flush separately
    assert_eq!(slots.confirmed_flush_lsn("logi"), Some(300));
}

#[test]
fn standby_feedback_reports_byte_lag() {
    // sender has shipped to sent_lsn; standby confirms write/flush/apply
    let fb = StandbyFeedback {
        sent_lsn: 1000,
        write_lsn: 980,
        flush_lsn: 950,
        apply_lsn: 900,
    };
    assert_eq!(fb.write_lag(), 20); // sent - write
    assert_eq!(fb.flush_lag(), 50); // sent - flush
    assert_eq!(fb.apply_lag(), 100); // sent - apply
    assert!(!fb.is_caught_up());

    let synced = StandbyFeedback {
        sent_lsn: 1000,
        write_lsn: 1000,
        flush_lsn: 1000,
        apply_lsn: 1000,
    };
    assert_eq!(synced.flush_lag(), 0);
    assert!(synced.is_caught_up());
}

#[test]
fn logical_decode_streams_only_committed_in_commit_order() {
    let mut rb = ReorderBuffer::new();

    // xact 5 starts, buffers two changes
    rb.begin(5, 10);
    rb.queue_change(5, Change::insert("users", vec!["1".into(), "alice".into()]));
    rb.queue_change(5, Change::insert("users", vec!["2".into(), "bob".into()]));

    // concurrent xact 6 starts and commits first
    rb.begin(6, 12);
    rb.queue_change(6, Change::delete("orders", vec!["7".into()]));
    let stream6 = rb.commit(6, 20);

    // xact 6's stream: BEGIN, its change, COMMIT — nothing from xact 5
    assert_eq!(stream6.len(), 3);
    assert!(matches!(stream6[0], DecodedChange::Begin { xid: 6, .. }));
    assert!(matches!(&stream6[1], DecodedChange::Delete { relation, .. } if relation == "orders"));
    assert!(matches!(stream6[2], DecodedChange::Commit { xid: 6, commit_lsn: 20 }));

    // now xact 5 commits — both its inserts surface, in insertion order
    let stream5 = rb.commit(5, 25);
    assert_eq!(stream5.len(), 4); // BEGIN + 2 inserts + COMMIT
    assert!(matches!(stream5[0], DecodedChange::Begin { xid: 5, .. }));
    assert!(
        matches!(&stream5[1], DecodedChange::Insert { tuple, .. } if tuple[1] == "alice")
    );
    assert!(
        matches!(&stream5[2], DecodedChange::Insert { tuple, .. } if tuple[1] == "bob")
    );
    assert!(matches!(stream5[3], DecodedChange::Commit { xid: 5, commit_lsn: 25 }));
}

#[test]
fn logical_decode_aborted_xact_emits_nothing() {
    let mut rb = ReorderBuffer::new();
    rb.begin(9, 30);
    rb.queue_change(9, Change::update("t", vec!["x".into()]));
    let out = rb.abort(9);
    assert!(out.is_empty());
    // committing an unknown/aborted xid yields nothing too
    assert!(rb.commit(9, 40).is_empty());
}

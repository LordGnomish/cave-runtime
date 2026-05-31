// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the EdgeHub reliable cloud-edge sync keeper.
//!
//! Faithful port of KubeEdge's reliable message delivery
//! (`edge/pkg/edgehub` SyncKeeper + the cloudcore `synccontroller`
//! ObjectSync resource-version model):
//!   - outbound messages are kept pending until ACKed by message ID;
//!   - an un-ACKed message becomes due for retransmission once a timeout
//!     elapses, and retransmitting bumps the timer + attempt counter;
//!   - inbound updates carry a per-resource resourceVersion; an update is
//!     applied only if strictly newer than what's already applied, so a
//!     re-delivered or out-of-order older message is idempotently dropped
//!     (eventual consistency).
//!
//! Pure delivery/merge logic — no WebSocket, no QUIC.

use cave_edge_runtime::edgehub::{EdgeHub, RecvOutcome, SyncMessage};

fn msg(id: u64, key: &str, rv: u64) -> SyncMessage {
    SyncMessage {
        id,
        resource_key: key.to_string(),
        resource_version: rv,
        payload: format!("payload-{id}"),
    }
}

// ─── outbound: pending / ACK ────────────────────────────────────────────────

#[test]
fn sent_message_is_pending_until_acked() {
    let mut hub = EdgeHub::new();
    hub.send(msg(1, "pod/default/web", 5), 0);
    assert!(hub.is_pending(1));
    assert_eq!(hub.pending_count(), 1);
    assert!(hub.ack(1));
    assert!(!hub.is_pending(1));
    assert_eq!(hub.pending_count(), 0);
}

#[test]
fn ack_of_unknown_id_returns_false() {
    let mut hub = EdgeHub::new();
    assert!(!hub.ack(999));
}

// ─── outbound: retransmission timer ─────────────────────────────────────────

#[test]
fn message_not_due_before_timeout() {
    let mut hub = EdgeHub::new();
    hub.send(msg(1, "k", 1), 100);
    // timeout = 10, now = 105 → only 5 elapsed, not due.
    assert!(hub.due_for_retransmit(105, 10).is_empty());
}

#[test]
fn message_due_after_timeout() {
    let mut hub = EdgeHub::new();
    hub.send(msg(1, "k", 1), 100);
    assert_eq!(hub.due_for_retransmit(110, 10), vec![1]);
}

#[test]
fn retransmit_resets_timer_and_counts_attempts() {
    let mut hub = EdgeHub::new();
    hub.send(msg(1, "k", 1), 100);
    assert_eq!(hub.due_for_retransmit(110, 10), vec![1]);
    hub.mark_retransmitted(1, 110);
    // Timer reset to 110 → not due again until 120.
    assert!(hub.due_for_retransmit(115, 10).is_empty());
    assert_eq!(hub.due_for_retransmit(120, 10), vec![1]);
    assert_eq!(hub.attempts(1), Some(2));
}

#[test]
fn due_returns_only_timed_out_ids_sorted() {
    let mut hub = EdgeHub::new();
    hub.send(msg(3, "k3", 1), 100);
    hub.send(msg(1, "k1", 1), 100);
    hub.send(msg(2, "k2", 1), 118); // sent later
    // At now=115, timeout=10: ids 1 and 3 are due (sent at 100), id 2 is not.
    assert_eq!(hub.due_for_retransmit(115, 10), vec![1, 3]);
}

// ─── inbound: resource-version merge (eventual consistency) ─────────────────

#[test]
fn first_update_for_a_key_is_applied() {
    let mut hub = EdgeHub::new();
    assert_eq!(hub.receive("pod/default/web", 5, "v5"), RecvOutcome::Applied);
    assert_eq!(hub.applied_version("pod/default/web"), Some(5));
}

#[test]
fn newer_update_is_applied() {
    let mut hub = EdgeHub::new();
    hub.receive("pod/default/web", 5, "v5");
    assert_eq!(hub.receive("pod/default/web", 8, "v8"), RecvOutcome::Applied);
    assert_eq!(hub.applied_version("pod/default/web"), Some(8));
}

#[test]
fn stale_or_duplicate_update_is_dropped_idempotently() {
    let mut hub = EdgeHub::new();
    hub.receive("pod/default/web", 8, "v8");
    // A re-delivered older message (rv 5) and an exact duplicate (rv 8) are
    // both dropped without regressing the applied version.
    assert_eq!(hub.receive("pod/default/web", 5, "v5"), RecvOutcome::Dropped);
    assert_eq!(hub.receive("pod/default/web", 8, "v8"), RecvOutcome::Dropped);
    assert_eq!(hub.applied_version("pod/default/web"), Some(8));
}

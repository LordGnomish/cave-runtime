// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! EdgeHub reliable cloud-edge sync keeper — KubeEdge `edge/pkg/edgehub`.
//!
//! Two halves of KubeEdge's reliable message delivery, ported as pure logic:
//!
//!   * **Outbound SyncKeeper.** Every message sent to the cloud is kept
//!     "pending" keyed by its message ID until an ACK arrives. An un-ACKed
//!     message becomes due for retransmission once a timeout elapses since its
//!     last send; retransmitting resets the timer and bumps an attempt
//!     counter. This is what lets the edge survive a flaky link without losing
//!     status reports.
//!
//!   * **Inbound ObjectSync merge.** Cloud→edge updates carry a per-resource
//!     `resourceVersion`. The edge applies an update only when it is strictly
//!     newer than the highest version already applied for that key; a
//!     re-delivered or out-of-order older message is dropped idempotently.
//!     Retransmission + monotonic-version-dedup together give the eventual
//!     consistency KubeEdge guarantees across reconnects.
//!
//! No transport — WebSocket/QUIC framing stays out of scope.

use std::collections::BTreeMap;

/// A message exchanged over the hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncMessage {
    /// Unique message ID (KubeEdge `Header.ID`).
    pub id: u64,
    /// The resource this message concerns (`type/namespace/name`).
    pub resource_key: String,
    /// Kubernetes resourceVersion carried by the update.
    pub resource_version: u64,
    pub payload: String,
}

/// Outcome of an inbound update after the resource-version merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvOutcome {
    /// The update was strictly newer and has been applied.
    Applied,
    /// The update was stale or a duplicate and was dropped.
    Dropped,
}

#[derive(Debug, Clone)]
struct Pending {
    msg: SyncMessage,
    last_send: u64,
    attempts: u32,
}

/// The reliable sync keeper.
#[derive(Debug, Clone, Default)]
pub struct EdgeHub {
    /// Outbound messages awaiting ACK, keyed by message ID.
    pending: BTreeMap<u64, Pending>,
    /// Highest applied resourceVersion per resource key.
    applied: BTreeMap<String, u64>,
}

impl EdgeHub {
    pub fn new() -> Self {
        Self::default()
    }

    // ── outbound ────────────────────────────────────────────────────────

    /// Send a message at time `now`; it is retained pending until ACKed.
    pub fn send(&mut self, msg: SyncMessage, now: u64) {
        self.pending.insert(
            msg.id,
            Pending {
                msg,
                last_send: now,
                attempts: 1,
            },
        );
    }

    /// Acknowledge a message; returns true if it was pending.
    pub fn ack(&mut self, id: u64) -> bool {
        self.pending.remove(&id).is_some()
    }

    pub fn is_pending(&self, id: u64) -> bool {
        self.pending.contains_key(&id)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Attempt count for a pending message (1 on first send).
    pub fn attempts(&self, id: u64) -> Option<u32> {
        self.pending.get(&id).map(|p| p.attempts)
    }

    /// IDs whose timeout has elapsed since their last send (`elapsed >=
    /// timeout`), sorted ascending.
    pub fn due_for_retransmit(&self, now: u64, timeout: u64) -> Vec<u64> {
        self.pending
            .values()
            .filter(|p| now.saturating_sub(p.last_send) >= timeout)
            .map(|p| p.msg.id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Record a retransmission at `now`: reset the timer and bump attempts.
    pub fn mark_retransmitted(&mut self, id: u64, now: u64) {
        if let Some(p) = self.pending.get_mut(&id) {
            p.last_send = now;
            p.attempts += 1;
        }
    }

    // ── inbound ─────────────────────────────────────────────────────────

    /// Merge an inbound update by resourceVersion. Applies only if strictly
    /// newer than the highest version already applied for the key.
    pub fn receive(&mut self, resource_key: &str, resource_version: u64, _payload: &str) -> RecvOutcome {
        match self.applied.get(resource_key) {
            Some(&cur) if resource_version <= cur => RecvOutcome::Dropped,
            _ => {
                self.applied.insert(resource_key.to_string(), resource_version);
                RecvOutcome::Applied
            }
        }
    }

    /// Highest applied resourceVersion for a key.
    pub fn applied_version(&self, resource_key: &str) -> Option<u64> {
        self.applied.get(resource_key).copied()
    }
}

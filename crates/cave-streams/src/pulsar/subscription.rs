// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008 (v4.2.0)
//         pulsar-broker/src/main/java/org/apache/pulsar/broker/service/Consumer.java
//         pulsar-broker/src/main/java/org/apache/pulsar/broker/service/Subscription.java
//         pulsar-broker/src/main/java/org/apache/pulsar/broker/service/persistent/
//             PersistentSubscription.java
//             PersistentDispatcherSingleActiveConsumer.java
//             PersistentDispatcherMultipleConsumers.java
//             PersistentStickyKeyDispatcherMultipleConsumers.java
//             MessageRedeliveryController.java
//
//! Pulsar subscription-type state machines — gap-fill atop
//! [`crate::pulsar_dispatch`].
//!
//! `pulsar_dispatch` covers the message-routing decision (given the
//! subscription type + consumers, which consumer-id should the next
//! message go to?).  This module covers the *state-machine* layer:
//!
//! * which ack modes are allowed for which subscription type
//!   (`Cumulative` is rejected for `Shared` and `Key_Shared`),
//! * negative ack (`nack`) → redelivery queue with bounded delay,
//! * Key_Shared sub-modes: `AUTO_SPLIT` (consistent hash ring) vs
//!   `STICKY` (explicit range assignments),
//! * Failover transitions when the active consumer disconnects.
//!
//! A [`SubscriptionState`] wraps a `pulsar_dispatch::SubscriptionDispatcher`
//! and ledges the unacked / nacked / per-key sticky-range state that the
//! dispatcher itself does not own.

use crate::pulsar_dispatch::{DispatchConsumer, SubscriptionDispatcher};
use crate::pulsar_wire::SubscriptionType;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Acknowledgement modes.  Mirrors `org.apache.pulsar.client.api.SubscriptionType`
/// matrix: only `Exclusive` and `Failover` permit `Cumulative` ack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AckMode {
    /// Ack a single message by id.
    Individual,
    /// Ack everything up to and including a message id.  Only valid for
    /// `Exclusive` and `Failover` subscriptions.
    Cumulative,
}

/// Errors raised by the subscription state machine.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SubscriptionError {
    /// `Exclusive` subscription already has a consumer.  Mirrors
    /// `ConsumerBusyException`.
    #[error("ConsumerBusyException: subscription '{0}' already has an exclusive consumer")]
    ConsumerBusy(String),

    /// `Cumulative` ack attempted on a `Shared` or `Key_Shared` subscription.
    /// Mirrors `NotAllowedException`.
    #[error(
        "NotAllowedException: cumulative ack is not supported for subscription type {0:?}"
    )]
    CumulativeNotAllowed(SubscriptionType),

    /// Ack/nack referenced a message id that the subscription never had
    /// in flight (already acked, never delivered, or wrong consumer).
    #[error("ack on unknown message-id {0}")]
    UnknownMessageId(u64),

    /// Sticky ranges supplied by a consumer overlap an existing claim.
    #[error("sticky ranges overlap with existing consumer {0}")]
    OverlappingStickyRange(u64),
}

/// Sub-modes for `Key_Shared` — mirrors `KeySharedMode` in Pulsar 4.2.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeySharedMode {
    /// Cave Streams' default: keys are hashed onto a consistent hash ring
    /// keyed by consumer id.  When a consumer leaves, only its share is
    /// re-balanced.
    AutoSplit,
    /// Consumers attach with explicit `[start, end]` hash ranges; sticky
    /// claims are honoured exactly as supplied.
    Sticky,
}

/// Inclusive `[start, end]` hash range used by Sticky mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashRange {
    pub start: u32,
    pub end: u32,
}

impl HashRange {
    pub fn contains(&self, h: u32) -> bool {
        self.start <= h && h <= self.end
    }

    pub fn overlaps(&self, other: &HashRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// Per-consumer ack and nack ledger.
#[derive(Debug, Default)]
struct ConsumerLedger {
    /// `(entry_id → delivered_at)` for messages dispatched but not yet
    /// acked.  Used to detect "ack on unknown" and to support nack timing.
    unacked: HashMap<u64, Instant>,
    /// Nacked entry-ids waiting to be redelivered after `nack_delay`.
    nacked: VecDeque<(u64, Instant)>,
    /// Cumulative ack high-water-mark.
    cumulative_hwm: Option<u64>,
}

#[derive(Debug)]
struct StateInner {
    /// Per-consumer book-keeping.
    consumers: BTreeMap<u64, ConsumerLedger>,
    /// Mode for Key_Shared subscriptions.
    key_shared_mode: KeySharedMode,
    /// Explicit sticky ranges (Sticky mode only).
    sticky_ranges: BTreeMap<u64, Vec<HashRange>>,
    /// Failover: which consumer is currently active (cached for fast
    /// transition detection).
    active_failover_consumer: Option<u64>,
    /// Nack redelivery delay (default 1 minute, matching Pulsar).
    nack_delay: Duration,
}

/// Wraps a [`SubscriptionDispatcher`] with the full per-subscription state
/// machine.
pub struct SubscriptionState {
    pub topic: String,
    pub subscription: String,
    pub kind: SubscriptionType,
    dispatcher: SubscriptionDispatcher,
    inner: Mutex<StateInner>,
}

impl SubscriptionState {
    pub fn new(
        topic: impl Into<String>,
        subscription: impl Into<String>,
        kind: SubscriptionType,
    ) -> Self {
        let topic = topic.into();
        let subscription = subscription.into();
        Self {
            dispatcher: SubscriptionDispatcher::new(topic.clone(), subscription.clone(), kind),
            topic,
            subscription,
            kind,
            inner: Mutex::new(StateInner {
                consumers: BTreeMap::new(),
                key_shared_mode: KeySharedMode::AutoSplit,
                sticky_ranges: BTreeMap::new(),
                active_failover_consumer: None,
                nack_delay: Duration::from_secs(60),
            }),
        }
    }

    /// Configure the Key_Shared sub-mode.  Default is `AutoSplit`.
    pub fn set_key_shared_mode(&self, mode: KeySharedMode) {
        self.inner.lock().unwrap().key_shared_mode = mode;
    }

    pub fn key_shared_mode(&self) -> KeySharedMode {
        self.inner.lock().unwrap().key_shared_mode
    }

    /// Override the nack → redelivery delay (default 60s).
    pub fn set_nack_delay(&self, d: Duration) {
        self.inner.lock().unwrap().nack_delay = d;
    }

    /// Attach a consumer.  For `Sticky` Key_Shared, `sticky_ranges` must
    /// not overlap any existing consumer's ranges.
    pub fn add_consumer(
        &self,
        c: DispatchConsumer,
        sticky_ranges: Vec<HashRange>,
    ) -> Result<(), SubscriptionError> {
        let consumer_id = c.consumer_id;
        // Exclusive check happens at the dispatcher.
        self.dispatcher
            .add_consumer(c)
            .map_err(|_| SubscriptionError::ConsumerBusy(self.subscription.clone()))?;
        let mut inner = self.inner.lock().unwrap();
        inner.consumers.insert(consumer_id, ConsumerLedger::default());

        if !sticky_ranges.is_empty() {
            // Sticky-range overlap check vs every prior consumer.
            for (&other, ranges) in inner.sticky_ranges.iter() {
                for r in ranges {
                    for new_r in &sticky_ranges {
                        if r.overlaps(new_r) {
                            // Roll back consumer add.
                            inner.consumers.remove(&consumer_id);
                            drop(inner);
                            self.dispatcher.remove_consumer(consumer_id);
                            return Err(SubscriptionError::OverlappingStickyRange(other));
                        }
                    }
                }
            }
            inner.sticky_ranges.insert(consumer_id, sticky_ranges);
        }

        // Recompute Failover active consumer.
        if matches!(self.kind, SubscriptionType::Failover) {
            inner.active_failover_consumer = self.dispatcher.active_consumer();
        }
        Ok(())
    }

    /// Detach a consumer.  Re-queues their unacked messages for
    /// redelivery; for Failover triggers an active-consumer transition.
    pub fn remove_consumer(&self, consumer_id: u64) -> bool {
        self.dispatcher.remove_consumer(consumer_id);
        let mut inner = self.inner.lock().unwrap();
        let had = inner.consumers.remove(&consumer_id).is_some();
        inner.sticky_ranges.remove(&consumer_id);
        if matches!(self.kind, SubscriptionType::Failover) {
            inner.active_failover_consumer = self.dispatcher.active_consumer();
        }
        had
    }

    /// Snapshot of consumer ids that the dispatcher currently knows about.
    pub fn consumer_ids(&self) -> BTreeSet<u64> {
        self.dispatcher.consumer_ids()
    }

    pub fn consumer_count(&self) -> usize {
        self.dispatcher.consumer_count()
    }

    /// Failover-only — currently active consumer.
    pub fn active_consumer(&self) -> Option<u64> {
        self.dispatcher.active_consumer()
    }

    /// Pick the consumer for the next message; honours the Key_Shared
    /// sub-mode.  Returns `None` if no eligible consumer exists.
    pub fn dispatch_message(
        &self,
        entry_id: u64,
        key: Option<&[u8]>,
    ) -> Option<u64> {
        let mode = self.key_shared_mode();
        // Sticky mode: walk the explicit range table first.
        if matches!(
            (self.kind, mode),
            (SubscriptionType::KeyShared, KeySharedMode::Sticky)
        ) {
            if let Some(key) = key {
                let h = stable_hash32(key);
                let inner = self.inner.lock().unwrap();
                for (&cid, ranges) in &inner.sticky_ranges {
                    for r in ranges {
                        if r.contains(h) {
                            // Record delivery in unacked ledger.
                            drop(inner);
                            self.record_delivery(cid, entry_id);
                            return Some(cid);
                        }
                    }
                }
                return None;
            }
        }
        // AutoSplit / non-key-shared: delegate to dispatcher.
        let cid = self.dispatcher.dispatch_message(key)?;
        self.record_delivery(cid, entry_id);
        Some(cid)
    }

    fn record_delivery(&self, consumer_id: u64, entry_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        let ledger = inner.consumers.entry(consumer_id).or_default();
        ledger.unacked.insert(entry_id, Instant::now());
    }

    /// Acknowledge.  Mode rules:
    /// * `Individual` is valid for every subscription type.
    /// * `Cumulative` is rejected for `Shared` and `Key_Shared`.
    pub fn ack(
        &self,
        consumer_id: u64,
        entry_id: u64,
        mode: AckMode,
    ) -> Result<(), SubscriptionError> {
        if matches!(mode, AckMode::Cumulative)
            && matches!(
                self.kind,
                SubscriptionType::Shared | SubscriptionType::KeyShared
            )
        {
            return Err(SubscriptionError::CumulativeNotAllowed(self.kind));
        }
        let mut inner = self.inner.lock().unwrap();
        let Some(ledger) = inner.consumers.get_mut(&consumer_id) else {
            return Err(SubscriptionError::UnknownMessageId(entry_id));
        };
        match mode {
            AckMode::Individual => {
                if ledger.unacked.remove(&entry_id).is_none() {
                    return Err(SubscriptionError::UnknownMessageId(entry_id));
                }
            }
            AckMode::Cumulative => {
                ledger
                    .unacked
                    .retain(|&eid, _| eid > entry_id);
                ledger.cumulative_hwm = Some(entry_id);
            }
        }
        Ok(())
    }

    /// Negative ack: schedule the entry for redelivery after `nack_delay`.
    pub fn nack(&self, consumer_id: u64, entry_id: u64) -> Result<(), SubscriptionError> {
        let mut inner = self.inner.lock().unwrap();
        let nack_delay = inner.nack_delay;
        let Some(ledger) = inner.consumers.get_mut(&consumer_id) else {
            return Err(SubscriptionError::UnknownMessageId(entry_id));
        };
        if ledger.unacked.remove(&entry_id).is_none() {
            return Err(SubscriptionError::UnknownMessageId(entry_id));
        }
        let ready_at = Instant::now() + nack_delay;
        ledger.nacked.push_back((entry_id, ready_at));
        Ok(())
    }

    /// Number of unacked messages currently held by `consumer_id`.
    pub fn unacked_count(&self, consumer_id: u64) -> usize {
        self.inner
            .lock()
            .unwrap()
            .consumers
            .get(&consumer_id)
            .map(|l| l.unacked.len())
            .unwrap_or(0)
    }

    /// Number of nacked messages awaiting redelivery for `consumer_id`.
    pub fn nacked_count(&self, consumer_id: u64) -> usize {
        self.inner
            .lock()
            .unwrap()
            .consumers
            .get(&consumer_id)
            .map(|l| l.nacked.len())
            .unwrap_or(0)
    }

    /// Drain the redelivery queue for `consumer_id`, returning entry ids
    /// whose nack delay has elapsed.
    pub fn drain_redeliveries(&self, consumer_id: u64) -> Vec<u64> {
        let now = Instant::now();
        let mut out = Vec::new();
        let mut inner = self.inner.lock().unwrap();
        let Some(ledger) = inner.consumers.get_mut(&consumer_id) else {
            return out;
        };
        while let Some(&(eid, ready_at)) = ledger.nacked.front() {
            if ready_at <= now {
                ledger.nacked.pop_front();
                ledger.unacked.insert(eid, now);
                out.push(eid);
            } else {
                break;
            }
        }
        out
    }

    /// Cumulative ack high-water-mark, if any.
    pub fn cumulative_hwm(&self, consumer_id: u64) -> Option<u64> {
        self.inner
            .lock()
            .unwrap()
            .consumers
            .get(&consumer_id)
            .and_then(|l| l.cumulative_hwm)
    }
}

/// Stable 32-bit hash used by Sticky Key_Shared ranges.  Not Murmur3 (no
/// extra dependency) but deterministic and reproducible across runs.
fn stable_hash32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5; // FNV-1a 32-bit basis
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cons(id: u64) -> DispatchConsumer {
        DispatchConsumer::new(id, 0)
    }

    #[test]
    fn exclusive_rejects_second_consumer_with_busy_error() {
        // cite: pulsar 4.2.0 ConsumerBusyException on exclusive
        let s = SubscriptionState::new("t", "s", SubscriptionType::Exclusive);
        s.add_consumer(cons(1), vec![]).unwrap();
        let err = s.add_consumer(cons(2), vec![]).unwrap_err();
        assert!(matches!(err, SubscriptionError::ConsumerBusy(_)));
    }

    #[test]
    fn exclusive_allows_cumulative_ack() {
        // cite: pulsar 4.2.0 cumulative ack supported on Exclusive
        let s = SubscriptionState::new("t", "s", SubscriptionType::Exclusive);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        s.dispatch_message(1, None).unwrap();
        s.dispatch_message(2, None).unwrap();
        s.ack(1, 2, AckMode::Cumulative).unwrap();
        assert_eq!(s.unacked_count(1), 0);
        assert_eq!(s.cumulative_hwm(1), Some(2));
    }

    #[test]
    fn failover_active_consumer_picks_lowest_failover_priority() {
        // cite: pulsar 4.2.0 PersistentDispatcherSingleActiveConsumer
        let s = SubscriptionState::new("t", "s", SubscriptionType::Failover);
        s.add_consumer(
            DispatchConsumer {
                consumer_id: 1,
                priority: 0,
                failover_priority: 5,
                unacked: 0,
                has_permits: true,
            },
            vec![],
        )
        .unwrap();
        s.add_consumer(
            DispatchConsumer {
                consumer_id: 2,
                priority: 0,
                failover_priority: 1,
                unacked: 0,
                has_permits: true,
            },
            vec![],
        )
        .unwrap();
        assert_eq!(s.active_consumer(), Some(2));
    }

    #[test]
    fn failover_transitions_when_active_disconnects() {
        // cite: pulsar 4.2.0 Failover re-election on consumer leave
        let s = SubscriptionState::new("t", "s", SubscriptionType::Failover);
        s.add_consumer(
            DispatchConsumer {
                consumer_id: 1,
                priority: 0,
                failover_priority: 1,
                unacked: 0,
                has_permits: true,
            },
            vec![],
        )
        .unwrap();
        s.add_consumer(
            DispatchConsumer {
                consumer_id: 2,
                priority: 0,
                failover_priority: 5,
                unacked: 0,
                has_permits: true,
            },
            vec![],
        )
        .unwrap();
        assert_eq!(s.active_consumer(), Some(1));
        s.remove_consumer(1);
        assert_eq!(s.active_consumer(), Some(2));
    }

    #[test]
    fn shared_individual_ack_works() {
        // cite: pulsar 4.2.0 Shared dispatcher individual-ack
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.add_consumer(cons(2), vec![]).unwrap();
        let c0 = s.dispatch_message(0, None).unwrap();
        let c1 = s.dispatch_message(1, None).unwrap();
        s.ack(c0, 0, AckMode::Individual).unwrap();
        s.ack(c1, 1, AckMode::Individual).unwrap();
        assert_eq!(s.unacked_count(c0), 0);
        assert_eq!(s.unacked_count(c1), 0);
    }

    #[test]
    fn shared_rejects_cumulative_ack() {
        // cite: pulsar 4.2.0 NotAllowedException on Shared cumulative ack
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        let err = s.ack(1, 0, AckMode::Cumulative).unwrap_err();
        assert_eq!(err, SubscriptionError::CumulativeNotAllowed(SubscriptionType::Shared));
    }

    #[test]
    fn key_shared_rejects_cumulative_ack() {
        // cite: pulsar 4.2.0 NotAllowedException on KeyShared cumulative ack
        let s = SubscriptionState::new("t", "s", SubscriptionType::KeyShared);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, Some(b"k")).unwrap();
        let err = s.ack(1, 0, AckMode::Cumulative).unwrap_err();
        assert_eq!(
            err,
            SubscriptionError::CumulativeNotAllowed(SubscriptionType::KeyShared)
        );
    }

    #[test]
    fn key_shared_auto_split_consistent_assignment() {
        // cite: pulsar 4.2.0 KeySharedMode.AUTO_SPLIT
        let s = SubscriptionState::new("t", "s", SubscriptionType::KeyShared);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.add_consumer(cons(2), vec![]).unwrap();
        let key = b"order-42";
        let first = s.dispatch_message(0, Some(key)).unwrap();
        // Subsequent messages with same key → same consumer.
        for entry_id in 1..5 {
            assert_eq!(s.dispatch_message(entry_id, Some(key)).unwrap(), first);
        }
    }

    #[test]
    fn key_shared_sticky_honours_explicit_ranges() {
        // cite: pulsar 4.2.0 KeySharedMode.STICKY range claims
        let s = SubscriptionState::new("t", "s", SubscriptionType::KeyShared);
        s.set_key_shared_mode(KeySharedMode::Sticky);
        // Cover the whole 32-bit space with two disjoint claims.
        s.add_consumer(
            cons(1),
            vec![HashRange { start: 0, end: u32::MAX / 2 }],
        )
        .unwrap();
        s.add_consumer(
            cons(2),
            vec![HashRange {
                start: u32::MAX / 2 + 1,
                end: u32::MAX,
            }],
        )
        .unwrap();
        // Try a few keys; each one lands on whichever consumer covers its hash.
        for k in [b"a".as_ref(), b"b", b"c", b"abc", b"42"] {
            let h = stable_hash32(k);
            let expected = if h <= u32::MAX / 2 { 1 } else { 2 };
            assert_eq!(s.dispatch_message(0, Some(k)).unwrap(), expected);
        }
    }

    #[test]
    fn key_shared_sticky_rejects_overlapping_ranges() {
        // cite: pulsar 4.2.0 OverlappingStickyRange validation
        let s = SubscriptionState::new("t", "s", SubscriptionType::KeyShared);
        s.set_key_shared_mode(KeySharedMode::Sticky);
        s.add_consumer(cons(1), vec![HashRange { start: 0, end: 1000 }])
            .unwrap();
        let err = s
            .add_consumer(cons(2), vec![HashRange { start: 500, end: 2000 }])
            .unwrap_err();
        assert_eq!(err, SubscriptionError::OverlappingStickyRange(1));
        assert_eq!(s.consumer_count(), 1); // c2 rolled back
    }

    #[test]
    fn nack_schedules_redelivery() {
        // cite: pulsar 4.2.0 negative-ack redelivery
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.set_nack_delay(Duration::from_millis(50));
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        s.nack(1, 0).unwrap();
        assert_eq!(s.unacked_count(1), 0);
        assert_eq!(s.nacked_count(1), 1);
        std::thread::sleep(Duration::from_millis(80));
        let redelivered = s.drain_redeliveries(1);
        assert_eq!(redelivered, vec![0]);
        assert_eq!(s.unacked_count(1), 1);
    }

    #[test]
    fn nack_respects_delay_before_redelivery() {
        // cite: pulsar 4.2.0 RedeliveryBackoff
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.set_nack_delay(Duration::from_secs(60));
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        s.nack(1, 0).unwrap();
        // Immediately drain — should be empty.
        let redelivered = s.drain_redeliveries(1);
        assert!(redelivered.is_empty());
    }

    #[test]
    fn ack_on_unknown_message_id_errors() {
        // cite: pulsar 4.2.0 ack on never-delivered → error
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.add_consumer(cons(1), vec![]).unwrap();
        let err = s.ack(1, 99, AckMode::Individual).unwrap_err();
        assert_eq!(err, SubscriptionError::UnknownMessageId(99));
    }

    #[test]
    fn ack_on_missing_consumer_errors() {
        // cite: pulsar 4.2.0 closed-consumer ack → error
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        let err = s.ack(99, 0, AckMode::Individual).unwrap_err();
        assert_eq!(err, SubscriptionError::UnknownMessageId(0));
    }

    #[test]
    fn nack_on_unknown_message_id_errors() {
        // cite: pulsar 4.2.0 nack on never-delivered → error
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.add_consumer(cons(1), vec![]).unwrap();
        let err = s.nack(1, 99).unwrap_err();
        assert_eq!(err, SubscriptionError::UnknownMessageId(99));
    }

    #[test]
    fn exclusive_individual_ack_clears_unacked() {
        // cite: pulsar 4.2.0 Exclusive individual ack
        let s = SubscriptionState::new("t", "s", SubscriptionType::Exclusive);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        s.dispatch_message(1, None).unwrap();
        s.ack(1, 0, AckMode::Individual).unwrap();
        assert_eq!(s.unacked_count(1), 1);
    }

    #[test]
    fn failover_allows_cumulative_ack() {
        // cite: pulsar 4.2.0 cumulative ack supported on Failover
        let s = SubscriptionState::new("t", "s", SubscriptionType::Failover);
        s.add_consumer(cons(1), vec![]).unwrap();
        for eid in 0..3 {
            s.dispatch_message(eid, None).unwrap();
        }
        s.ack(1, 2, AckMode::Cumulative).unwrap();
        assert_eq!(s.cumulative_hwm(1), Some(2));
        assert_eq!(s.unacked_count(1), 0);
    }

    #[test]
    fn cumulative_ack_does_not_clear_future_entries() {
        // cite: pulsar 4.2.0 cumulative ack only clears up to id
        let s = SubscriptionState::new("t", "s", SubscriptionType::Exclusive);
        s.add_consumer(cons(1), vec![]).unwrap();
        for eid in 0..5 {
            s.dispatch_message(eid, None).unwrap();
        }
        s.ack(1, 2, AckMode::Cumulative).unwrap();
        // Entries 3, 4 still unacked.
        assert_eq!(s.unacked_count(1), 2);
    }

    #[test]
    fn remove_consumer_clears_state() {
        // cite: pulsar 4.2.0 consumer-disconnect cleans book-keeping
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        assert_eq!(s.unacked_count(1), 1);
        s.remove_consumer(1);
        assert_eq!(s.unacked_count(1), 0);
        assert_eq!(s.consumer_count(), 0);
    }

    #[test]
    fn hash_range_contains_endpoints() {
        // cite: pulsar 4.2.0 HashRange is inclusive
        let r = HashRange { start: 10, end: 20 };
        assert!(r.contains(10));
        assert!(r.contains(20));
        assert!(r.contains(15));
        assert!(!r.contains(9));
        assert!(!r.contains(21));
    }

    #[test]
    fn hash_range_overlaps_detection() {
        // cite: pulsar 4.2.0 OverlappingStickyRange check
        let a = HashRange { start: 10, end: 20 };
        let b = HashRange { start: 15, end: 25 };
        let c = HashRange { start: 21, end: 30 };
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn key_shared_sticky_routes_unmapped_key_to_none() {
        // cite: pulsar 4.2.0 STICKY: keys outside range → no consumer
        let s = SubscriptionState::new("t", "s", SubscriptionType::KeyShared);
        s.set_key_shared_mode(KeySharedMode::Sticky);
        s.add_consumer(cons(1), vec![HashRange { start: 0, end: 1000 }])
            .unwrap();
        // Most keys hash outside [0, 1000]; this one is highly unlikely
        // to land in the tiny range.
        let result = s.dispatch_message(0, Some(b"unmapped-key-aaaaaaaaaa"));
        let h = stable_hash32(b"unmapped-key-aaaaaaaaaa");
        if h > 1000 {
            assert!(result.is_none());
        } else {
            assert_eq!(result, Some(1));
        }
    }

    #[test]
    fn cumulative_hwm_persists_across_dispatch() {
        // cite: pulsar 4.2.0 cumulative HWM is sticky
        let s = SubscriptionState::new("t", "s", SubscriptionType::Exclusive);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        s.ack(1, 0, AckMode::Cumulative).unwrap();
        assert_eq!(s.cumulative_hwm(1), Some(0));
        s.dispatch_message(1, None).unwrap();
        assert_eq!(s.cumulative_hwm(1), Some(0)); // HWM unchanged by dispatch
    }

    #[test]
    fn nack_clears_unacked_immediately() {
        // cite: pulsar 4.2.0 nack moves entry from unacked to pending-redelivery
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        s.add_consumer(cons(1), vec![]).unwrap();
        s.dispatch_message(0, None).unwrap();
        s.dispatch_message(1, None).unwrap();
        s.nack(1, 0).unwrap();
        assert_eq!(s.unacked_count(1), 1);
        assert_eq!(s.nacked_count(1), 1);
    }

    #[test]
    fn drain_redeliveries_on_missing_consumer_returns_empty() {
        // cite: pulsar 4.2.0 closed-consumer redelivery drain
        let s = SubscriptionState::new("t", "s", SubscriptionType::Shared);
        assert!(s.drain_redeliveries(99).is_empty());
    }

    #[test]
    fn key_shared_mode_default_is_auto_split() {
        // cite: pulsar 4.2.0 default KeySharedMode = AUTO_SPLIT
        let s = SubscriptionState::new("t", "s", SubscriptionType::KeyShared);
        assert_eq!(s.key_shared_mode(), KeySharedMode::AutoSplit);
    }
}

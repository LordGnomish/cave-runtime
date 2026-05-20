// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulsar subscription-type dispatchers.
//!
//! For each subscription type [`SubscriptionType`], this module owns the
//! per-topic membership ledger and a `dispatch_message` routine that
//! returns the *consumer-id* a single message should be delivered to.
//! Cave Streams uses these dispatchers to demultiplex incoming
//! `CommandMessage`s without touching the wire layer.
//!
//! Mirrors Apache Pulsar 4.2.0
//!   `pulsar-broker/src/main/java/org/apache/pulsar/broker/service/persistent/PersistentDispatcherSingleActiveConsumer.java`
//!   `pulsar-broker/src/main/java/org/apache/pulsar/broker/service/persistent/PersistentStickyKeyDispatcherMultipleConsumers.java`
//!   `pulsar-broker/src/main/java/org/apache/pulsar/broker/service/persistent/PersistentDispatcherMultipleConsumers.java`

use crate::error::{StreamsError, StreamsResult};
use crate::pulsar_wire::SubscriptionType;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

/// Per-subscription consumer entry tracked by the dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchConsumer {
    pub consumer_id: u64,
    /// Stable sort key — Pulsar uses the consumer's join order; cave-streams
    /// uses the consumer-id which is monotonic per session.
    pub priority: u64,
    /// Failover only: a higher `failover_priority` wins active-consumer
    /// election (lower = higher priority, matching Pulsar).
    pub failover_priority: i32,
    /// Number of unacked messages in flight to this consumer (used by
    /// the Shared dispatcher for round-robin among ready consumers).
    pub unacked: u32,
    /// `true` when the consumer has positive flow permits.
    pub has_permits: bool,
}

impl DispatchConsumer {
    pub fn new(consumer_id: u64, priority: u64) -> Self {
        Self {
            consumer_id,
            priority,
            failover_priority: 0,
            unacked: 0,
            has_permits: true,
        }
    }
}

/// Per-(tenant, namespace, topic, subscription) dispatch state.
pub struct SubscriptionDispatcher {
    pub topic: String,
    pub subscription: String,
    pub kind: SubscriptionType,
    inner: Mutex<DispatcherInner>,
}

#[derive(Debug, Default)]
struct DispatcherInner {
    consumers: BTreeMap<u64, DispatchConsumer>,
    /// Round-robin pointer for the Shared dispatcher.
    rr_index: usize,
    /// Sticky key → consumer_id for the Key_Shared dispatcher (consistent
    /// hashing keeps ordering for a given key while a consumer survives).
    key_owner: HashMap<Vec<u8>, u64>,
}

impl SubscriptionDispatcher {
    pub fn new(
        topic: impl Into<String>,
        subscription: impl Into<String>,
        kind: SubscriptionType,
    ) -> Self {
        Self {
            topic: topic.into(),
            subscription: subscription.into(),
            kind,
            inner: Mutex::new(DispatcherInner::default()),
        }
    }

    pub fn add_consumer(&self, c: DispatchConsumer) -> StreamsResult<()> {
        let mut inner = self.inner.lock().unwrap();
        if matches!(self.kind, SubscriptionType::Exclusive) && !inner.consumers.is_empty() {
            return Err(StreamsError::Internal(format!(
                "ConsumerBusyException: {:?} already has an exclusive consumer",
                self.subscription
            )));
        }
        inner.consumers.insert(c.consumer_id, c);
        Ok(())
    }

    pub fn remove_consumer(&self, consumer_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.consumers.remove(&consumer_id);
        inner.key_owner.retain(|_, owner| *owner != consumer_id);
    }

    pub fn consumer_count(&self) -> usize {
        self.inner.lock().unwrap().consumers.len()
    }

    /// Pick the next consumer for a single message.  Returns `None` if no
    /// eligible consumer exists.  `key` is only consulted by Key_Shared.
    pub fn dispatch_message(&self, key: Option<&[u8]>) -> Option<u64> {
        let mut inner = self.inner.lock().unwrap();
        if inner.consumers.is_empty() {
            return None;
        }
        match self.kind {
            SubscriptionType::Exclusive => {
                // Single consumer: deliver to it iff it has permits.
                inner
                    .consumers
                    .values()
                    .find(|c| c.has_permits)
                    .map(|c| c.consumer_id)
            }
            SubscriptionType::Failover => {
                // Active consumer = the one with the *lowest*
                // failover_priority (ties broken by lowest consumer_id).
                let active = inner
                    .consumers
                    .values()
                    .filter(|c| c.has_permits)
                    .min_by(|a, b| {
                        a.failover_priority
                            .cmp(&b.failover_priority)
                            .then(a.consumer_id.cmp(&b.consumer_id))
                    })
                    .map(|c| c.consumer_id);
                active
            }
            SubscriptionType::Shared => {
                // Round-robin among consumers with permits.
                let ids: Vec<u64> = inner
                    .consumers
                    .values()
                    .filter(|c| c.has_permits)
                    .map(|c| c.consumer_id)
                    .collect();
                if ids.is_empty() {
                    return None;
                }
                let idx = inner.rr_index % ids.len();
                inner.rr_index = inner.rr_index.wrapping_add(1);
                Some(ids[idx])
            }
            SubscriptionType::KeyShared => {
                let key = key?;
                // Consistent assignment: hash the key to one of the
                // consumers; lock that mapping so subsequent messages with
                // the same key go to the same consumer (preserving order).
                if let Some(owner) = inner.key_owner.get(key) {
                    let still_present = inner.consumers.contains_key(owner);
                    if still_present {
                        return Some(*owner);
                    }
                    inner.key_owner.remove(key);
                }
                // Hash → bucket among current consumer IDs.
                let mut ids: Vec<u64> = inner.consumers.keys().copied().collect();
                ids.sort();
                if ids.is_empty() {
                    return None;
                }
                let h = stable_hash(key);
                let pick = ids[(h as usize) % ids.len()];
                inner.key_owner.insert(key.to_vec(), pick);
                Some(pick)
            }
        }
    }

    /// Snapshot of all consumer IDs (sorted).
    pub fn consumer_ids(&self) -> BTreeSet<u64> {
        self.inner
            .lock()
            .unwrap()
            .consumers
            .keys()
            .copied()
            .collect()
    }

    /// For Failover only: returns the currently-active consumer.
    pub fn active_consumer(&self) -> Option<u64> {
        if !matches!(self.kind, SubscriptionType::Failover) {
            return None;
        }
        let inner = self.inner.lock().unwrap();
        inner
            .consumers
            .values()
            .min_by(|a, b| {
                a.failover_priority
                    .cmp(&b.failover_priority)
                    .then(a.consumer_id.cmp(&b.consumer_id))
            })
            .map(|c| c.consumer_id)
    }

    /// Inspect Key_Shared sticky-key ownership table.
    pub fn key_owner(&self, key: &[u8]) -> Option<u64> {
        self.inner.lock().unwrap().key_owner.get(key).copied()
    }
}

/// Stable, allocation-free hash for [`SubscriptionType::KeyShared`].
/// Mirrors Pulsar's `Murmur3_32Hash` only in *stability* — a real
/// implementation would use Murmur3 to interop with a Pulsar client.
fn stable_hash(bytes: &[u8]) -> u64 {
    // FNV-1a 64-bit — deterministic and dependency-free.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

// ─────────────────────────────────────────────────────────────────────────
// Pulsar dispatch tests — feat/cave-streams-deeper-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(tenant_id: &str, suffix: &str) -> String {
        format!("persistent://{}/ns/{}", tenant_id, suffix)
    }

    #[test]
    fn test_dispatch_exclusive_rejects_second_consumer() {
        // cite: pulsar 4.2.0 .../service/persistent/PersistentSubscription#addConsumer (Exclusive)
        let tenant_id = "pd-001";
        let d =
            SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::Exclusive);
        d.add_consumer(DispatchConsumer::new(1, 0)).unwrap();
        let err = d.add_consumer(DispatchConsumer::new(2, 0));
        assert!(err.is_err());
        assert_eq!(d.consumer_count(), 1);
    }

    #[test]
    fn test_dispatch_exclusive_returns_single_consumer() {
        // cite: pulsar 4.2.0 PersistentDispatcherSingleActiveConsumer
        let tenant_id = "pd-002";
        let d =
            SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::Exclusive);
        d.add_consumer(DispatchConsumer::new(7, 0)).unwrap();
        assert_eq!(d.dispatch_message(None), Some(7));
    }

    #[test]
    fn test_dispatch_failover_picks_lowest_priority() {
        // cite: pulsar 4.2.0 PersistentDispatcherSingleActiveConsumer (Failover priority)
        let tenant_id = "pd-003";
        let d =
            SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::Failover);
        d.add_consumer(DispatchConsumer {
            consumer_id: 1,
            priority: 0,
            failover_priority: 5,
            unacked: 0,
            has_permits: true,
        })
        .unwrap();
        d.add_consumer(DispatchConsumer {
            consumer_id: 2,
            priority: 0,
            failover_priority: 1, // wins
            unacked: 0,
            has_permits: true,
        })
        .unwrap();
        assert_eq!(d.active_consumer(), Some(2));
        assert_eq!(d.dispatch_message(None), Some(2));
    }

    #[test]
    fn test_dispatch_failover_falls_over_when_active_lacks_permits() {
        // cite: pulsar 4.2.0 active consumer must have permits to receive
        let tenant_id = "pd-004";
        let d =
            SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::Failover);
        d.add_consumer(DispatchConsumer {
            consumer_id: 1,
            priority: 0,
            failover_priority: 1,
            unacked: 0,
            has_permits: false, // no permits
        })
        .unwrap();
        d.add_consumer(DispatchConsumer {
            consumer_id: 2,
            priority: 0,
            failover_priority: 5,
            unacked: 0,
            has_permits: true,
        })
        .unwrap();
        // dispatch_message must skip the no-permit primary
        assert_eq!(d.dispatch_message(None), Some(2));
    }

    #[test]
    fn test_dispatch_shared_round_robin() {
        // cite: pulsar 4.2.0 PersistentDispatcherMultipleConsumers (round-robin)
        let tenant_id = "pd-005";
        let d = SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::Shared);
        for id in [10u64, 20, 30] {
            d.add_consumer(DispatchConsumer::new(id, 0)).unwrap();
        }
        let p1 = d.dispatch_message(None).unwrap();
        let p2 = d.dispatch_message(None).unwrap();
        let p3 = d.dispatch_message(None).unwrap();
        let p4 = d.dispatch_message(None).unwrap();
        let mut got = vec![p1, p2, p3];
        got.sort();
        assert_eq!(got, vec![10, 20, 30]);
        assert_eq!(p4, p1, "RR wraps around");
    }

    #[test]
    fn test_dispatch_shared_skips_no_permit_consumers() {
        // cite: pulsar 4.2.0 (skip consumers without permits)
        let tenant_id = "pd-006";
        let d = SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::Shared);
        d.add_consumer(DispatchConsumer {
            consumer_id: 1,
            priority: 0,
            failover_priority: 0,
            unacked: 0,
            has_permits: false,
        })
        .unwrap();
        d.add_consumer(DispatchConsumer::new(2, 0)).unwrap();
        for _ in 0..3 {
            assert_eq!(d.dispatch_message(None), Some(2));
        }
    }

    #[test]
    fn test_dispatch_key_shared_sticky() {
        // cite: pulsar 4.2.0 PersistentStickyKeyDispatcherMultipleConsumers
        let tenant_id = "pd-007";
        let d =
            SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::KeyShared);
        d.add_consumer(DispatchConsumer::new(1, 0)).unwrap();
        d.add_consumer(DispatchConsumer::new(2, 0)).unwrap();
        let key = b"order-42";
        let owner = d.dispatch_message(Some(key)).unwrap();
        for _ in 0..5 {
            assert_eq!(d.dispatch_message(Some(key)).unwrap(), owner);
        }
    }

    #[test]
    fn test_dispatch_key_shared_rebalances_on_consumer_loss() {
        // cite: pulsar 4.2.0 (ownership transfers when owner disconnects)
        let tenant_id = "pd-008";
        let d =
            SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::KeyShared);
        d.add_consumer(DispatchConsumer::new(1, 0)).unwrap();
        d.add_consumer(DispatchConsumer::new(2, 0)).unwrap();
        let key = b"k";
        let first = d.dispatch_message(Some(key)).unwrap();
        d.remove_consumer(first);
        let second = d.dispatch_message(Some(key)).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn test_dispatch_no_consumers_returns_none() {
        // cite: pulsar 4.2.0 (dispatch returns null with no consumers)
        let tenant_id = "pd-009";
        let d = SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::Shared);
        assert_eq!(d.dispatch_message(None), None);
    }

    #[test]
    fn test_dispatch_remove_consumer_clears_key_owner() {
        // cite: pulsar 4.2.0 KeyShared (cleanup when consumer leaves)
        let tenant_id = "pd-010";
        let d =
            SubscriptionDispatcher::new(topic(tenant_id, "t"), "sub", SubscriptionType::KeyShared);
        d.add_consumer(DispatchConsumer::new(1, 0)).unwrap();
        let key = b"hot-key";
        let owner = d.dispatch_message(Some(key)).unwrap();
        d.remove_consumer(owner);
        assert!(d.key_owner(key).is_none());
    }
}

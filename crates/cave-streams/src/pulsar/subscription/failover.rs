// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/.../service/persistent/PersistentDispatcherSingleActiveConsumer.java

//! Failover subscription — one *active* consumer at a time, chosen by
//! priority; other consumers stand by.

use super::{PolicyConsumer, SubscriptionPolicy};
use crate::error::StreamsResult;
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct FailoverPolicy {
    consumers: BTreeMap<u64, PolicyConsumer>,
}

impl FailoverPolicy {
    /// Currently-active consumer id (lowest priority + lowest id with
    /// permits).  Returns `None` if all consumers lack permits.
    pub fn active_consumer(&self) -> Option<u64> {
        self.consumers
            .values()
            .filter(|c| c.has_permits)
            .min_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then(a.consumer_id.cmp(&b.consumer_id))
            })
            .map(|c| c.consumer_id)
    }
}

impl SubscriptionPolicy for FailoverPolicy {
    fn add_consumer(&mut self, c: PolicyConsumer) -> StreamsResult<()> {
        self.consumers.insert(c.consumer_id, c);
        Ok(())
    }

    fn remove_consumer(&mut self, consumer_id: u64) {
        self.consumers.remove(&consumer_id);
    }

    fn pick(&mut self, _key: Option<&[u8]>) -> Option<u64> {
        self.active_consumer()
    }

    fn consumer_count(&self) -> usize {
        self.consumers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failover_picks_lowest_priority() {
        // cite: pulsar 4.2.0 PersistentDispatcherSingleActiveConsumer priority
        // ensemble = fo-001
        let mut p = FailoverPolicy::default();
        p.add_consumer(PolicyConsumer {
            consumer_id: 1,
            priority: 5,
            has_permits: true,
        })
        .unwrap();
        p.add_consumer(PolicyConsumer {
            consumer_id: 2,
            priority: 1, // wins
            has_permits: true,
        })
        .unwrap();
        assert_eq!(p.pick(None), Some(2));
    }

    #[test]
    fn test_failover_ties_broken_by_consumer_id() {
        // cite: pulsar 4.2.0 ties broken by join order (lowest id in cave-streams)
        // ensemble = fo-002
        let mut p = FailoverPolicy::default();
        p.add_consumer(PolicyConsumer {
            consumer_id: 7,
            priority: 0,
            has_permits: true,
        })
        .unwrap();
        p.add_consumer(PolicyConsumer {
            consumer_id: 2,
            priority: 0,
            has_permits: true,
        })
        .unwrap();
        assert_eq!(p.pick(None), Some(2));
    }

    #[test]
    fn test_failover_skips_active_without_permits() {
        // cite: pulsar 4.2.0 active falls over when no permits
        // ensemble = fo-003
        let mut p = FailoverPolicy::default();
        p.add_consumer(PolicyConsumer {
            consumer_id: 1,
            priority: 0,
            has_permits: false,
        })
        .unwrap();
        p.add_consumer(PolicyConsumer::new(2)).unwrap();
        assert_eq!(p.pick(None), Some(2));
    }

    #[test]
    fn test_failover_failover_on_remove() {
        // cite: pulsar 4.2.0 active changes when previous active leaves
        // ensemble = fo-004
        let mut p = FailoverPolicy::default();
        p.add_consumer(PolicyConsumer {
            consumer_id: 1,
            priority: 1,
            has_permits: true,
        })
        .unwrap();
        p.add_consumer(PolicyConsumer {
            consumer_id: 2,
            priority: 5,
            has_permits: true,
        })
        .unwrap();
        assert_eq!(p.pick(None), Some(1));
        p.remove_consumer(1);
        assert_eq!(p.pick(None), Some(2));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/.../service/persistent/PersistentDispatcherMultipleConsumers.java

//! Shared subscription — round-robin among consumers with permits.

use super::{PolicyConsumer, SubscriptionPolicy};
use crate::error::StreamsResult;
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct SharedPolicy {
    consumers: BTreeMap<u64, PolicyConsumer>,
    rr_index: usize,
}

impl SubscriptionPolicy for SharedPolicy {
    fn add_consumer(&mut self, c: PolicyConsumer) -> StreamsResult<()> {
        self.consumers.insert(c.consumer_id, c);
        Ok(())
    }

    fn remove_consumer(&mut self, consumer_id: u64) {
        self.consumers.remove(&consumer_id);
    }

    fn pick(&mut self, _key: Option<&[u8]>) -> Option<u64> {
        let ready: Vec<u64> = self
            .consumers
            .values()
            .filter(|c| c.has_permits)
            .map(|c| c.consumer_id)
            .collect();
        if ready.is_empty() {
            return None;
        }
        let idx = self.rr_index % ready.len();
        self.rr_index = self.rr_index.wrapping_add(1);
        Some(ready[idx])
    }

    fn consumer_count(&self) -> usize {
        self.consumers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_round_robin_distributes() {
        // cite: pulsar 4.2.0 PersistentDispatcherMultipleConsumers round-robin
        // ensemble = sh-001
        let mut p = SharedPolicy::default();
        for id in [1u64, 2, 3] {
            p.add_consumer(PolicyConsumer::new(id)).unwrap();
        }
        let mut picks: Vec<u64> = (0..3).map(|_| p.pick(None).unwrap()).collect();
        picks.sort();
        assert_eq!(picks, vec![1, 2, 3]);
    }

    #[test]
    fn test_shared_wraps_after_full_pass() {
        // cite: pulsar 4.2.0 round-robin wraps
        // ensemble = sh-002
        let mut p = SharedPolicy::default();
        for id in [1u64, 2] {
            p.add_consumer(PolicyConsumer::new(id)).unwrap();
        }
        let a = p.pick(None).unwrap();
        let b = p.pick(None).unwrap();
        let c = p.pick(None).unwrap();
        assert_eq!(a, c, "wraps");
        assert_ne!(a, b);
    }

    #[test]
    fn test_shared_skips_consumer_without_permits() {
        // cite: pulsar 4.2.0 only consumers with permits eligible
        // ensemble = sh-003
        let mut p = SharedPolicy::default();
        p.add_consumer(PolicyConsumer {
            consumer_id: 1,
            priority: 0,
            has_permits: false,
        })
        .unwrap();
        p.add_consumer(PolicyConsumer::new(2)).unwrap();
        for _ in 0..3 {
            assert_eq!(p.pick(None), Some(2));
        }
    }

    #[test]
    fn test_shared_returns_none_when_empty() {
        // cite: pulsar 4.2.0 dispatch with no eligible returns null
        // ensemble = sh-004
        let mut p = SharedPolicy::default();
        assert_eq!(p.pick(None), None);
    }

    #[test]
    fn test_shared_remove_drops_consumer() {
        // cite: pulsar 4.2.0 consumer disconnect cleanup
        // ensemble = sh-005
        let mut p = SharedPolicy::default();
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        p.add_consumer(PolicyConsumer::new(2)).unwrap();
        p.remove_consumer(1);
        assert_eq!(p.consumer_count(), 1);
        assert_eq!(p.pick(None), Some(2));
    }
}

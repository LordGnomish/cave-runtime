// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/.../service/persistent/PersistentDispatcherSingleActiveConsumer.java

//! Exclusive subscription — exactly one active consumer at a time.

use super::{PolicyConsumer, SubscriptionPolicy};
use crate::error::{StreamsError, StreamsResult};

#[derive(Debug, Default)]
pub struct ExclusivePolicy {
    consumer: Option<PolicyConsumer>,
}

impl SubscriptionPolicy for ExclusivePolicy {
    fn add_consumer(&mut self, c: PolicyConsumer) -> StreamsResult<()> {
        if self.consumer.is_some() {
            return Err(StreamsError::Internal(
                "ConsumerBusyException: Exclusive already attached".into(),
            ));
        }
        self.consumer = Some(c);
        Ok(())
    }

    fn remove_consumer(&mut self, consumer_id: u64) {
        if let Some(c) = &self.consumer {
            if c.consumer_id == consumer_id {
                self.consumer = None;
            }
        }
    }

    fn pick(&mut self, _key: Option<&[u8]>) -> Option<u64> {
        self.consumer
            .as_ref()
            .filter(|c| c.has_permits)
            .map(|c| c.consumer_id)
    }

    fn consumer_count(&self) -> usize {
        usize::from(self.consumer.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exclusive_accepts_first_consumer() {
        // cite: pulsar 4.2.0 PersistentSubscription Exclusive accept
        // ensemble = ex-001
        let mut p = ExclusivePolicy::default();
        assert!(p.add_consumer(PolicyConsumer::new(1)).is_ok());
        assert_eq!(p.consumer_count(), 1);
    }

    #[test]
    fn test_exclusive_rejects_second_consumer() {
        // cite: pulsar 4.2.0 ConsumerBusyException
        // ensemble = ex-002
        let mut p = ExclusivePolicy::default();
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        let err = p.add_consumer(PolicyConsumer::new(2));
        assert!(err.is_err());
        assert_eq!(p.consumer_count(), 1);
    }

    #[test]
    fn test_exclusive_pick_returns_the_only_consumer() {
        // cite: pulsar 4.2.0 PersistentDispatcherSingleActiveConsumer dispatch
        // ensemble = ex-003
        let mut p = ExclusivePolicy::default();
        p.add_consumer(PolicyConsumer::new(42)).unwrap();
        assert_eq!(p.pick(None), Some(42));
    }

    #[test]
    fn test_exclusive_pick_skips_when_no_permits() {
        // cite: pulsar 4.2.0 consumer without flow permits not eligible
        // ensemble = ex-004
        let mut p = ExclusivePolicy::default();
        p.add_consumer(PolicyConsumer {
            consumer_id: 1,
            priority: 0,
            has_permits: false,
        })
        .unwrap();
        assert_eq!(p.pick(None), None);
    }

    #[test]
    fn test_exclusive_remove_lets_new_consumer_join() {
        // cite: pulsar 4.2.0 Exclusive: detach allows new attach
        // ensemble = ex-005
        let mut p = ExclusivePolicy::default();
        p.add_consumer(PolicyConsumer::new(1)).unwrap();
        p.remove_consumer(1);
        assert_eq!(p.consumer_count(), 0);
        assert!(p.add_consumer(PolicyConsumer::new(2)).is_ok());
    }
}

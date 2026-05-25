// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory channel implementation — `pkg/reconciler/channel/in_memory`.
//!
//! The IMC (In-Memory Channel) is Knative Eventing's default channel
//! transport. It buffers CloudEvents in a per-channel ring queue and
//! fans them out to subscribers with at-least-once delivery + per-
//! destination retry counters. Upstream emits an Addressable URI
//! (`http://imc-dispatcher.<ns>.svc.cluster.local`) that producers POST
//! events to; the dispatcher then drains its queues round-robin.
//!
//! This module ports the dispatcher / queue / retry logic. The HTTP
//! receive surface is a separate axum route in `routes.rs` (Phase 3).

use crate::eventing::Subscription;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Minimal CloudEvent v1.0 envelope (subset matching `eventing_transports`).
#[derive(Debug, Clone)]
pub struct CloudEvent {
    pub id: String,
    pub source: String,
    pub event_type: String,
    pub data: Vec<u8>,
    pub partition_key: Option<String>,
}

/// One subscriber's queue + retry book-keeping.
#[derive(Debug, Default)]
struct SubscriberQueue {
    events: VecDeque<(CloudEvent, u32)>, // (event, attempt count)
    delivered: u64,
    failed: u64,
}

/// Backoff strategy — linear or exponential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backoff {
    Linear,
    Exponential,
}

/// Channel-level dispatcher config — mirrors upstream `Config.Delivery`.
#[derive(Debug, Clone)]
pub struct ImcConfig {
    pub max_attempts: u32,
    pub base_backoff: Duration,
    pub backoff: Backoff,
    pub queue_capacity: usize,
    pub dead_letter_sink: Option<String>,
}

impl Default for ImcConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_backoff: Duration::from_millis(200),
            backoff: Backoff::Exponential,
            queue_capacity: 1024,
            dead_letter_sink: None,
        }
    }
}

/// In-memory channel — one logical Knative `Channel` CR.
pub struct InMemoryChannel {
    pub name: String,
    pub namespace: String,
    pub config: ImcConfig,
    queues: HashMap<String, SubscriberQueue>,
    dlq: VecDeque<(String, CloudEvent)>,
    created_at: Instant,
}

impl InMemoryChannel {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            config: ImcConfig::default(),
            queues: HashMap::new(),
            dlq: VecDeque::new(),
            created_at: Instant::now(),
        }
    }

    pub fn with_config(mut self, config: ImcConfig) -> Self {
        self.config = config;
        self
    }

    /// Addressable URI per upstream `imc-dispatcher.<ns>.svc.cluster.local`.
    pub fn addressable_uri(&self) -> String {
        format!(
            "http://imc-dispatcher.{}.svc.cluster.local/{}",
            self.namespace, self.name
        )
    }

    /// Subscribe a delivery target — initializes its per-subscriber queue.
    pub fn subscribe(&mut self, sub: &Subscription) {
        self.queues
            .entry(sub.uid.clone())
            .or_insert_with(SubscriberQueue::default);
    }

    pub fn unsubscribe(&mut self, uid: &str) {
        self.queues.remove(uid);
    }

    /// Publish an event — fan-out enqueues into every subscriber's queue.
    /// Drops the event on a subscriber whose queue is full (returns the per-sub overflow count).
    pub fn publish(&mut self, event: CloudEvent) -> usize {
        let cap = self.config.queue_capacity;
        let mut overflow = 0usize;
        for q in self.queues.values_mut() {
            if q.events.len() >= cap {
                overflow += 1;
                continue;
            }
            q.events.push_back((event.clone(), 0));
        }
        overflow
    }

    /// Drain one event from each subscriber and feed it to `deliver`.
    /// On `Ok` increments delivered; on `Err` increments attempts and re-enqueues unless attempts == max.
    /// Returns `(delivered, failed_terminal)`.
    pub fn dispatch_round<F: FnMut(&str, &CloudEvent) -> Result<(), String>>(
        &mut self,
        mut deliver: F,
    ) -> (u32, u32) {
        let mut delivered = 0u32;
        let mut terminal = 0u32;
        let mut to_dlq: Vec<(String, CloudEvent)> = Vec::new();
        for (uid, q) in self.queues.iter_mut() {
            let Some((event, attempts)) = q.events.pop_front() else {
                continue;
            };
            match deliver(uid, &event) {
                Ok(()) => {
                    q.delivered += 1;
                    delivered += 1;
                }
                Err(_) => {
                    let next = attempts + 1;
                    if next >= self.config.max_attempts {
                        q.failed += 1;
                        terminal += 1;
                        to_dlq.push((uid.clone(), event));
                    } else {
                        q.events.push_back((event, next));
                    }
                }
            }
        }
        for (uid, ev) in to_dlq {
            self.dlq.push_back((uid, ev));
        }
        (delivered, terminal)
    }

    /// Backoff duration for the Nth attempt (1-indexed).
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        match self.config.backoff {
            Backoff::Linear => self.config.base_backoff * attempt,
            Backoff::Exponential => {
                let mult = 1u32 << attempt.min(10).saturating_sub(1);
                self.config.base_backoff * mult.max(1)
            }
        }
    }

    pub fn queue_depth(&self, uid: &str) -> usize {
        self.queues.get(uid).map(|q| q.events.len()).unwrap_or(0)
    }

    pub fn delivered_count(&self, uid: &str) -> u64 {
        self.queues.get(uid).map(|q| q.delivered).unwrap_or(0)
    }

    pub fn failed_count(&self, uid: &str) -> u64 {
        self.queues.get(uid).map(|q| q.failed).unwrap_or(0)
    }

    pub fn dead_letter_depth(&self) -> usize {
        self.dlq.len()
    }

    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    pub fn subscriber_uids(&self) -> Vec<String> {
        let mut uids: Vec<String> = self.queues.keys().cloned().collect();
        uids.sort();
        uids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eventing::Subscription;

    fn ev(id: &str) -> CloudEvent {
        CloudEvent {
            id: id.into(),
            source: "/test".into(),
            event_type: "demo.test".into(),
            data: b"hi".to_vec(),
            partition_key: None,
        }
    }

    fn sub(uid: &str) -> Subscription {
        Subscription {
            uid: uid.into(),
            subscriber_uri: format!("http://{}/", uid),
            reply_uri: None,
        }
    }

    #[test]
    fn addressable_uri_uses_namespace_and_name() {
        let imc = InMemoryChannel::new("ns", "my-channel");
        assert_eq!(
            imc.addressable_uri(),
            "http://imc-dispatcher.ns.svc.cluster.local/my-channel"
        );
    }

    #[test]
    fn publish_fans_out_to_all_subscribers() {
        let mut imc = InMemoryChannel::new("ns", "c");
        imc.subscribe(&sub("a"));
        imc.subscribe(&sub("b"));
        imc.subscribe(&sub("c"));
        let overflow = imc.publish(ev("e1"));
        assert_eq!(overflow, 0);
        assert_eq!(imc.queue_depth("a"), 1);
        assert_eq!(imc.queue_depth("b"), 1);
        assert_eq!(imc.queue_depth("c"), 1);
    }

    #[test]
    fn dispatch_round_marks_delivered() {
        let mut imc = InMemoryChannel::new("ns", "c");
        imc.subscribe(&sub("a"));
        imc.publish(ev("e1"));
        let (d, t) = imc.dispatch_round(|_uid, _e| Ok(()));
        assert_eq!(d, 1);
        assert_eq!(t, 0);
        assert_eq!(imc.delivered_count("a"), 1);
        assert_eq!(imc.queue_depth("a"), 0);
    }

    #[test]
    fn dispatch_retries_until_max_then_dlq() {
        let mut imc = InMemoryChannel::new("ns", "c");
        imc.config.max_attempts = 3;
        imc.subscribe(&sub("a"));
        imc.publish(ev("e1"));
        for _ in 0..3 {
            let (d, _t) = imc.dispatch_round(|_, _| Err("boom".into()));
            assert_eq!(d, 0);
        }
        // After 3 failed attempts, event lands in DLQ.
        assert_eq!(imc.queue_depth("a"), 0);
        assert_eq!(imc.failed_count("a"), 1);
        assert_eq!(imc.dead_letter_depth(), 1);
    }

    #[test]
    fn dispatch_succeeds_on_second_attempt() {
        let mut imc = InMemoryChannel::new("ns", "c");
        imc.config.max_attempts = 5;
        imc.subscribe(&sub("a"));
        imc.publish(ev("e1"));
        let (d, _) = imc.dispatch_round(|_, _| Err("transient".into()));
        assert_eq!(d, 0);
        let (d, _) = imc.dispatch_round(|_, _| Ok(()));
        assert_eq!(d, 1);
        assert_eq!(imc.delivered_count("a"), 1);
        assert_eq!(imc.dead_letter_depth(), 0);
    }

    #[test]
    fn publish_overflow_drops_when_queue_full() {
        let mut imc = InMemoryChannel::new("ns", "c").with_config(ImcConfig {
            queue_capacity: 2,
            ..ImcConfig::default()
        });
        imc.subscribe(&sub("a"));
        imc.publish(ev("e1"));
        imc.publish(ev("e2"));
        let overflow = imc.publish(ev("e3"));
        assert_eq!(overflow, 1);
        assert_eq!(imc.queue_depth("a"), 2);
    }

    #[test]
    fn backoff_exponential_doubles_per_attempt() {
        let imc = InMemoryChannel::new("ns", "c");
        assert_eq!(imc.backoff_for_attempt(1), Duration::from_millis(200));
        assert_eq!(imc.backoff_for_attempt(2), Duration::from_millis(400));
        assert_eq!(imc.backoff_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn backoff_linear_scales_per_attempt() {
        let mut imc = InMemoryChannel::new("ns", "c");
        imc.config.backoff = Backoff::Linear;
        assert_eq!(imc.backoff_for_attempt(1), Duration::from_millis(200));
        assert_eq!(imc.backoff_for_attempt(3), Duration::from_millis(600));
        assert_eq!(imc.backoff_for_attempt(5), Duration::from_millis(1000));
    }

    #[test]
    fn unsubscribe_drops_queue() {
        let mut imc = InMemoryChannel::new("ns", "c");
        imc.subscribe(&sub("a"));
        imc.publish(ev("e1"));
        imc.unsubscribe("a");
        assert_eq!(imc.queue_depth("a"), 0);
        assert!(imc.subscriber_uids().is_empty());
    }

    #[test]
    fn publish_to_zero_subscribers_is_no_op() {
        let mut imc = InMemoryChannel::new("ns", "c");
        let overflow = imc.publish(ev("orphan"));
        assert_eq!(overflow, 0);
    }
}

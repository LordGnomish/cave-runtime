// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Event bus bridge — KubeEdge `edge/pkg/eventbus`.
//!
//! The eventbus is the seam between the MQTT world (devices speak MQTT on the
//! `$hw/events/...` tree) and the edge core's internal message bus. This port
//! provides the two pieces that are pure logic:
//!
//!   * `topic_matches` — MQTT v3.1.1 topic-filter matching, including the `+`
//!     single-level and `#` multi-level wildcards, the rule that `#` also
//!     matches the parent level, and the guard that a leading wildcard never
//!     matches a `$`-prefixed system topic;
//!   * `EventBus::classify` — mapping KubeEdge's well-known topics onto the
//!     internal message kinds the device twin / membership modules consume.
//!
//! Delivery uses an in-process FIFO queue per subscription — the cave-streams
//! local-queue stand-in for the broker hop, so the routing can be exercised
//! without an MQTT broker or sockets.

use std::collections::BTreeMap;
use std::collections::VecDeque;

/// A message flowing across the bus: the concrete topic and its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub topic: String,
    pub payload: String,
}

/// Classification of a KubeEdge well-known topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeTopicKind {
    /// `$hw/events/node/<id>/membership/...`
    Membership,
    /// `$hw/events/device/<id>/twin/...`
    TwinUpdate,
    /// `$hw/events/device/<id>/state/update`
    DeviceStateUpdate,
    /// `$hw/events/upload/#`
    Upload,
    /// Anything outside the KubeEdge taxonomy.
    Unknown,
}

/// Opaque subscription handle.
pub type SubId = usize;

/// Split a topic / filter into levels.
fn levels(s: &str) -> Vec<&str> {
    s.split('/').collect()
}

/// MQTT v3.1.1 topic-filter match: does `filter` match concrete `topic`?
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    let f = levels(filter);
    let t = levels(topic);

    // `$`-topic guard: a filter beginning with a wildcard must not match a
    // topic whose first level is a `$`-prefixed system topic.
    if let Some(first_topic) = t.first() {
        if first_topic.starts_with('$') {
            if let Some(first_filter) = f.first() {
                if *first_filter == "#" || *first_filter == "+" {
                    return false;
                }
            }
        }
    }

    let mut fi = 0;
    let mut ti = 0;
    while fi < f.len() {
        match f[fi] {
            "#" => {
                // Multi-level wildcard: matches the remainder, including the
                // parent level (zero remaining levels is a match). Must be the
                // last level in a valid filter.
                return true;
            }
            "+" => {
                // Single-level wildcard: consume exactly one topic level.
                if ti >= t.len() {
                    return false;
                }
                fi += 1;
                ti += 1;
            }
            literal => {
                if ti >= t.len() || t[ti] != literal {
                    return false;
                }
                fi += 1;
                ti += 1;
            }
        }
    }
    // All filter levels consumed: match iff the topic is also exhausted.
    ti == t.len()
}

struct Subscription {
    filter: String,
    queue: VecDeque<Message>,
}

/// The event bus: a set of subscriptions, each backed by a local FIFO queue.
#[derive(Default)]
pub struct EventBus {
    subs: BTreeMap<SubId, Subscription>,
    next_id: SubId,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            subs: BTreeMap::new(),
            next_id: 0,
        }
    }

    /// Classify a concrete topic against the KubeEdge `$hw/events/...` tree.
    pub fn classify(topic: &str) -> EdgeTopicKind {
        let l = levels(topic);
        // $hw / events / <kind> / <id> / <op> [/ ...]
        if l.len() >= 3 && l[0] == "$hw" && l[1] == "events" {
            match l[2] {
                "node" if l.iter().any(|s| *s == "membership") => EdgeTopicKind::Membership,
                "device" => {
                    if l.iter().any(|s| *s == "twin") {
                        EdgeTopicKind::TwinUpdate
                    } else if l.iter().any(|s| *s == "state") {
                        EdgeTopicKind::DeviceStateUpdate
                    } else {
                        EdgeTopicKind::Unknown
                    }
                }
                "upload" => EdgeTopicKind::Upload,
                _ => EdgeTopicKind::Unknown,
            }
        } else {
            EdgeTopicKind::Unknown
        }
    }

    /// Register a subscription for `filter`; returns its handle.
    pub fn subscribe(&mut self, filter: &str) -> SubId {
        let id = self.next_id;
        self.next_id += 1;
        self.subs.insert(
            id,
            Subscription {
                filter: filter.to_string(),
                queue: VecDeque::new(),
            },
        );
        id
    }

    /// Publish `payload` to `topic`; fan out a copy to every matching queue.
    pub fn publish(&mut self, topic: &str, payload: &str) {
        for sub in self.subs.values_mut() {
            if topic_matches(&sub.filter, topic) {
                sub.queue.push_back(Message {
                    topic: topic.to_string(),
                    payload: payload.to_string(),
                });
            }
        }
    }

    /// Dequeue the next payload for a subscription (FIFO).
    pub fn poll(&mut self, sub: SubId) -> Option<String> {
        self.subs
            .get_mut(&sub)
            .and_then(|s| s.queue.pop_front())
            .map(|m| m.payload)
    }

    /// Dequeue the next full message for a subscription (FIFO).
    pub fn poll_message(&mut self, sub: SubId) -> Option<Message> {
        self.subs.get_mut(&sub).and_then(|s| s.queue.pop_front())
    }

    /// Pending message count for a subscription.
    pub fn depth(&self, sub: SubId) -> usize {
        self.subs.get(&sub).map(|s| s.queue.len()).unwrap_or(0)
    }
}

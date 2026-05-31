// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the eventbus MQTT bridge.
//!
//! Faithful port of:
//!   - the MQTT v3.1.1 topic-filter matching rules (`+` single level, `#`
//!     multi level incl. the parent, and the `$`-topic wildcard guard) that
//!     KubeEdge `edge/pkg/eventbus` relies on for its `$hw/events/...` tree;
//!   - the eventbus → internal classification of KubeEdge's well-known topics;
//!   - a cave-streams-style local FIFO queue fan-out (subscribe / publish /
//!     poll), the in-process stand-in for the MQTT broker hop.
//!
//! Pure routing logic — no MQTT broker, no sockets.

use cave_edge_runtime::eventbus::{EdgeTopicKind, EventBus, topic_matches};

// ─── MQTT topic-filter matching ─────────────────────────────────────────────

#[test]
fn exact_topic_matches() {
    assert!(topic_matches("a/b/c", "a/b/c"));
    assert!(!topic_matches("a/b/c", "a/b/d"));
}

#[test]
fn plus_matches_exactly_one_level() {
    assert!(topic_matches("a/+/c", "a/b/c"));
    assert!(!topic_matches("a/+/c", "a/b/d/c"));
    assert!(!topic_matches("a/+/c", "a/c"));
}

#[test]
fn hash_matches_many_levels() {
    assert!(topic_matches("a/#", "a/b/c"));
    assert!(topic_matches("a/#", "a/b"));
}

#[test]
fn hash_matches_the_parent_level() {
    // Per spec, `sport/#` also matches `sport` itself.
    assert!(topic_matches("sport/#", "sport"));
}

#[test]
fn leading_wildcard_does_not_match_dollar_topic() {
    // MQTT: a wildcard at the first level must not match `$`-prefixed topics.
    assert!(!topic_matches("#", "$hw/events/x"));
    assert!(!topic_matches("+/events/x", "$hw/events/x"));
}

#[test]
fn explicit_dollar_prefix_matches_dollar_topic() {
    assert!(topic_matches("$hw/#", "$hw/events/device/d1/twin/update"));
}

// ─── KubeEdge topic classification ──────────────────────────────────────────

#[test]
fn classify_twin_update_topic() {
    assert_eq!(
        EventBus::classify("$hw/events/device/dev1/twin/update"),
        EdgeTopicKind::TwinUpdate
    );
}

#[test]
fn classify_membership_topic() {
    assert_eq!(
        EventBus::classify("$hw/events/node/node1/membership/get"),
        EdgeTopicKind::Membership
    );
}

#[test]
fn classify_device_state_topic() {
    assert_eq!(
        EventBus::classify("$hw/events/device/dev1/state/update"),
        EdgeTopicKind::DeviceStateUpdate
    );
}

#[test]
fn classify_unknown_topic() {
    assert_eq!(
        EventBus::classify("random/topic"),
        EdgeTopicKind::Unknown
    );
}

// ─── local FIFO queue fan-out ───────────────────────────────────────────────

#[test]
fn subscriber_receives_matching_publish_in_fifo_order() {
    let mut bus = EventBus::new();
    let sub = bus.subscribe("a/+/c");
    bus.publish("a/b/c", "first");
    bus.publish("a/x/c", "second");
    assert_eq!(bus.poll(sub).as_deref(), Some("first"));
    assert_eq!(bus.poll(sub).as_deref(), Some("second"));
    assert_eq!(bus.poll(sub), None);
}

#[test]
fn non_matching_publish_is_not_delivered() {
    let mut bus = EventBus::new();
    let sub = bus.subscribe("a/+/c");
    bus.publish("a/b/d", "nope");
    assert_eq!(bus.poll(sub), None);
}

#[test]
fn multiple_subscribers_each_get_a_copy() {
    let mut bus = EventBus::new();
    let s1 = bus.subscribe("dev/#");
    let s2 = bus.subscribe("dev/temp");
    bus.publish("dev/temp", "23.5");
    assert_eq!(bus.poll(s1).as_deref(), Some("23.5"));
    assert_eq!(bus.poll(s2).as_deref(), Some("23.5"));
}

#[test]
fn depth_reports_pending_messages() {
    let mut bus = EventBus::new();
    let sub = bus.subscribe("x/#");
    bus.publish("x/1", "a");
    bus.publish("x/2", "b");
    assert_eq!(bus.depth(sub), 2);
    let _ = bus.poll(sub);
    assert_eq!(bus.depth(sub), 1);
}

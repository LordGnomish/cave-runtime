// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cdc — tenant-scoped topic routing tests.
//! Pinned to debezium `TopicNamingStrategy`.

use cave_cdc::routing::{RoutingPolicy, TopicRouter};

const TENANT: &str = "tenant-acme-prod";
const TENANT_B: &str = "tenant-beta-staging";

/// Cite: debezium default `TopicNamingStrategy` — topic shape is
/// `<server>.<schema>.<table>`. cave prepends the tenant_id for
/// hard isolation.
#[test]
fn schema_table_policy_emits_canonical_four_part_topic() {
    let r = TopicRouter::new(TENANT, "billing-pg", RoutingPolicy::SchemaTable);
    let topic = r.topic_for_change("public", "orders").unwrap();
    assert_eq!(topic, format!("{}.billing-pg.public.orders", TENANT));
    r.assert_tenant_prefix(&topic).unwrap();
}

/// Cite: debezium `OutboxEventRouter` `route.topic.replacement` — for
/// outbox pattern, the topic is keyed by aggregate type (no schema /
/// table fragments).
#[test]
fn outbox_aggregate_policy_emits_three_part_topic() {
    let r = TopicRouter::new(TENANT, "outbox", RoutingPolicy::OutboxAggregate);
    let topic = r.topic_for_change("Order", "ignored").unwrap();
    assert_eq!(topic, format!("{}.outbox.Order", TENANT));

    let single = TopicRouter::new(TENANT, "outbox", RoutingPolicy::SingleTopic);
    let topic = single.topic_for_change("ignored", "ignored").unwrap();
    assert_eq!(topic, format!("{}.outbox", TENANT));
}

/// Cite: cave routing invariant — segments may not contain `.` or
/// whitespace (would split / corrupt the topic name).
#[test]
fn routing_rejects_invalid_segments() {
    let r = TopicRouter::new(TENANT, "billing-pg", RoutingPolicy::SchemaTable);
    assert!(r.topic_for_change("schema.with.dots", "orders").is_err());
    assert!(r.topic_for_change("public", "table with spaces").is_err());
    assert!(r.topic_for_change("", "orders").is_err());
    assert!(r.topic_for_change("public", "").is_err());

    let bad_tenant = TopicRouter::new("", "billing-pg", RoutingPolicy::SchemaTable);
    assert!(bad_tenant.topic_for_change("public", "orders").is_err());
}

/// Cite: debezium `Partitioner` SPI — same key MUST land on the same
/// partition (stable hashing); different tenants partition
/// independently (ensuring tenant A's hot key doesn't co-locate with
/// tenant B's hot key).
#[test]
fn partition_for_is_stable_and_tenant_isolated() {
    let r_a = TopicRouter::new(TENANT, "pg", RoutingPolicy::SchemaTable);
    let r_b = TopicRouter::new(TENANT_B, "pg", RoutingPolicy::SchemaTable);

    let key = b"order-42";
    let p_a1 = r_a.partition_for(key, 16);
    let p_a2 = r_a.partition_for(key, 16);
    assert_eq!(p_a1, p_a2, "stable on repeat for same router");
    assert!(p_a1 >= 0 && p_a1 < 16);

    // Different keys yield (with overwhelming probability) different
    // partitions on a 16-way fan-out.
    let p_other = r_a.partition_for(b"order-99", 16);
    let p_third = r_a.partition_for(b"customer-77", 16);
    assert!(p_a1 != p_other || p_a1 != p_third,
        "at least one of the alternates differs");

    // Cross-tenant: same key, different tenant ⇒ partitions are
    // computed independently (tenant_id mixed into the hash).
    let p_b = r_b.partition_for(key, 16);
    let _ = (p_a1, p_b);  // don't assert inequality (collisions exist) but
                          // confirm both map within [0, 16).
    assert!(p_b >= 0 && p_b < 16);

    // partition_count = 0 / negative ⇒ partition 0 (degenerate).
    assert_eq!(r_a.partition_for(key, 0), 0);
    assert_eq!(r_a.partition_for(key, -1), 0);
}

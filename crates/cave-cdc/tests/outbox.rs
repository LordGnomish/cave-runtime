//! cave-cdc — transactional outbox event router tests.
//! Pinned to debezium `OutboxEventRouter`.

use cave_cdc::{CdcError, OutboxEntry, OutboxEventRouter};

const TENANT: &str = "tenant-acme-prod";

fn entry(id: &str, agg_type: &str, agg_id: &str) -> OutboxEntry {
    OutboxEntry {
        id: id.into(),
        tenant_id: TENANT.into(),
        aggregate_type: agg_type.into(),
        aggregate_id: agg_id.into(),
        event_type: "OrderCreated".into(),
        payload: serde_json::json!({ "amount_usd_cents": 9999 }),
        created_at: chrono::Utc::now(),
    }
}

/// Cite: debezium `OutboxEventRouter::apply` — entries map onto a
/// `<prefix>.<tenant>.<aggregate_type>` topic with the aggregate id
/// as the key, and `id` / `eventType` / `tenant_id` as headers.
#[test]
fn outbox_routing_topic_key_and_headers() {
    let mut r = OutboxEventRouter::new(TENANT, "outbox");
    let routed = r.route(&entry("evt-1", "Order", "order-7")).unwrap();
    assert_eq!(routed.topic, format!("outbox.{}.Order", TENANT));
    assert_eq!(routed.key, "order-7");
    assert_eq!(routed.headers.get("id").map(|s| s.as_str()), Some("evt-1"));
    assert_eq!(routed.headers.get("eventType").map(|s| s.as_str()), Some("OrderCreated"));
    assert_eq!(routed.headers.get("tenant_id").map(|s| s.as_str()), Some(TENANT));
    assert_eq!(routed.value, serde_json::json!({ "amount_usd_cents": 9999 }));
}

/// Cite: debezium `OutboxEventRouter::apply` — duplicate `id`
/// detection. Replaying the same id ⇒ DuplicateOutboxEventId.
#[test]
fn duplicate_outbox_id_is_rejected_with_typed_error() {
    let mut r = OutboxEventRouter::new(TENANT, "outbox");
    r.route(&entry("evt-1", "Order", "order-7")).unwrap();
    let err = r.route(&entry("evt-1", "Order", "order-9")).unwrap_err();
    assert_eq!(err, CdcError::DuplicateOutboxEventId("evt-1".into()));
    assert_eq!(r.dedupe_size(), 1);

    // Operator-driven cleanup re-opens the id.
    assert!(r.forget("evt-1"));
    assert!(!r.seen("evt-1"));
    r.route(&entry("evt-1", "Order", "order-9")).unwrap();
}

/// Cite: cave multi-tenant invariant — an entry whose tenant_id does
/// not match the router's is rejected before any side-effect.
#[test]
fn cross_tenant_outbox_entry_is_rejected() {
    let mut r = OutboxEventRouter::new(TENANT, "outbox");
    let mut foreign = entry("evt-2", "Order", "order-1");
    foreign.tenant_id = "tenant-other".into();
    let err = r.route(&foreign).unwrap_err();
    assert!(matches!(err, CdcError::CrossTenantDenied { .. }));
    assert_eq!(r.dedupe_size(), 0, "rejected entry not added to dedupe set");
}

/// Cite: debezium `OutboxEventRouter::apply` — required-field
/// validation: id, aggregate_type, aggregate_id, tenant_id MUST all
/// be non-empty.
#[test]
fn outbox_entry_required_fields_are_enforced() {
    let mut r = OutboxEventRouter::new(TENANT, "outbox");
    for missing in [
        OutboxEntry { id: "".into(), ..entry("x", "Order", "o-1") },
        OutboxEntry { aggregate_type: "".into(), ..entry("evt", "Order", "o-1") },
        OutboxEntry { aggregate_id: "".into(), ..entry("evt", "Order", "o-1") },
        OutboxEntry { tenant_id: "".into(), ..entry("evt", "Order", "o-1") },
    ] {
        assert!(r.route(&missing).is_err());
    }
}

/// Cite: debezium `OutboxEventRouter` — different aggregate types
/// route onto different topics; multiple events per type co-exist.
#[test]
fn multi_aggregate_routing_yields_distinct_topics() {
    let mut r = OutboxEventRouter::new(TENANT, "outbox");
    let order_route = r.route(&entry("e-1", "Order", "o-1")).unwrap();
    let payment_route = r.route(&entry("e-2", "Payment", "p-1")).unwrap();
    let refund_route = r.route(&entry("e-3", "Refund", "r-1")).unwrap();
    assert_eq!(order_route.topic,   format!("outbox.{}.Order", TENANT));
    assert_eq!(payment_route.topic, format!("outbox.{}.Payment", TENANT));
    assert_eq!(refund_route.topic,  format!("outbox.{}.Refund", TENANT));
    assert_eq!(r.dedupe_size(), 3);
}

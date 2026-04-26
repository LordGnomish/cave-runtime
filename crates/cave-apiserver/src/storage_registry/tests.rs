//! storage_registry tests — strategy, consistent reads, watch progress,
//! streaming list, tenant index.

use super::*;
use serde_json::json;

fn ctx(tenant: &str) -> StrategyContext {
    StrategyContext { user: "alice".into(), tenant_id: tenant.into(), namespace: "default".into() }
}

// ─────────────────────────────────────────────────────────────────────────────
// DefaultStrategy — `store_test.go::TestStrategy_*`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn strategy_namespace_scoped() {
    let s = DefaultStrategy { namespaced: true };
    assert!(s.namespace_scoped());
    let s = DefaultStrategy { namespaced: false };
    assert!(!s.namespace_scoped());
}

#[test]
fn strategy_prepare_for_create_stamps_metadata() {
    let s = DefaultStrategy { namespaced: true };
    let mut o = json!({"metadata": {"name": "p1"}});
    s.prepare_for_create(&ctx("acme"), &mut o);
    assert!(o["metadata"]["creationTimestamp"].is_string());
    assert_eq!(o["metadata"]["resourceVersion"], "0");
}

#[test]
fn strategy_prepare_for_create_inserts_tenant_annotation() {
    let s = DefaultStrategy { namespaced: true };
    let mut o = json!({"metadata": {"name": "p1"}});
    s.prepare_for_create(&ctx("acme"), &mut o);
    assert_eq!(o["metadata"]["annotations"]["cave.runtime/tenant-id"], "acme");
}

#[test]
fn strategy_prepare_for_create_preserves_existing_tenant() {
    let s = DefaultStrategy { namespaced: true };
    let mut o = json!({"metadata": {"name": "p1",
        "annotations": {"cave.runtime/tenant-id": "globex"}}});
    s.prepare_for_create(&ctx("acme"), &mut o);
    assert_eq!(o["metadata"]["annotations"]["cave.runtime/tenant-id"], "globex",
        "existing tenant annotation preserved (validate will catch mismatch)");
}

#[test]
fn strategy_prepare_for_update_preserves_immutable_fields() {
    let s = DefaultStrategy { namespaced: true };
    let old = json!({"metadata": {"name": "p1", "creationTimestamp": "2025-01-01T00:00:00Z", "uid": "abc"}});
    let mut new = json!({"metadata": {"name": "p1", "creationTimestamp": "2026-01-01T00:00:00Z", "uid": "different"}});
    s.prepare_for_update(&ctx("acme"), &mut new, &old);
    assert_eq!(new["metadata"]["creationTimestamp"], "2025-01-01T00:00:00Z");
    assert_eq!(new["metadata"]["uid"], "abc");
}

#[test]
fn strategy_validate_rejects_missing_metadata() {
    let s = DefaultStrategy { namespaced: true };
    let o = json!({"spec": {}});
    matches!(s.validate(&ctx("acme"), &o), Err(StrategyError::Invalid(_)));
}

#[test]
fn strategy_validate_rejects_empty_name() {
    let s = DefaultStrategy { namespaced: true };
    let o = json!({"metadata": {"name": ""}});
    matches!(s.validate(&ctx("acme"), &o), Err(StrategyError::Invalid(_)));
}

#[test]
fn strategy_validate_rejects_invalid_dns_name() {
    let s = DefaultStrategy { namespaced: true };
    let o = json!({"metadata": {"name": "BAD_NAME"}});
    matches!(s.validate(&ctx("acme"), &o), Err(StrategyError::Invalid(_)));
}

#[test]
fn strategy_validate_accepts_valid_dns_name() {
    let s = DefaultStrategy { namespaced: true };
    let o = json!({"metadata": {"name": "my-pod-123"}});
    assert!(s.validate(&ctx("acme"), &o).is_ok());
}

#[test]
fn strategy_validate_rejects_cross_tenant_annotation() {
    let s = DefaultStrategy { namespaced: true };
    let o = json!({"metadata": {"name": "p1",
        "annotations": {"cave.runtime/tenant-id": "globex"}}});
    matches!(s.validate(&ctx("acme"), &o), Err(StrategyError::Forbidden(_)));
}

#[test]
fn strategy_validate_update_rejects_tenant_change() {
    let s = DefaultStrategy { namespaced: true };
    let old = json!({"metadata": {"name": "p1",
        "annotations": {"cave.runtime/tenant-id": "acme"}}});
    let new = json!({"metadata": {"name": "p1",
        "annotations": {"cave.runtime/tenant-id": "globex"}}});
    matches!(s.validate_update(&ctx("globex"), &new, &old), Err(StrategyError::Forbidden(_)));
}

// ─────────────────────────────────────────────────────────────────────────────
// is_dns1123_subdomain — `validation/util.go::TestIsDNS1123Subdomain`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dns_subdomain_basic_ok() {
    assert!(is_dns1123_subdomain("my-app"));
    assert!(is_dns1123_subdomain("my.app.example"));
    assert!(is_dns1123_subdomain("a"));
}

#[test]
fn dns_subdomain_rejects_uppercase() {
    assert!(!is_dns1123_subdomain("MyApp"));
}

#[test]
fn dns_subdomain_rejects_empty() {
    assert!(!is_dns1123_subdomain(""));
}

#[test]
fn dns_subdomain_rejects_too_long() {
    let long: String = "a".repeat(254);
    assert!(!is_dns1123_subdomain(&long));
}

#[test]
fn dns_subdomain_rejects_leading_dash() {
    assert!(!is_dns1123_subdomain("-foo"));
}

#[test]
fn dns_subdomain_rejects_trailing_dash() {
    assert!(!is_dns1123_subdomain("foo-"));
}

#[test]
fn dns_subdomain_rejects_consecutive_dots() {
    assert!(!is_dns1123_subdomain("foo..bar"));
}

#[test]
fn dns_subdomain_rejects_underscore() {
    assert!(!is_dns1123_subdomain("foo_bar"));
}

// ─────────────────────────────────────────────────────────────────────────────
// evaluate_consistent_read — KEP-956
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ccr_no_rv_serves_from_cache() {
    let r = evaluate_consistent_read(None, 100);
    assert_eq!(r, ConsistentReadOutcome::ServeFromCache { at_rv: 100 });
}

#[test]
fn ccr_zero_rv_serves_from_cache() {
    assert_eq!(evaluate_consistent_read(Some("0"), 100),
               ConsistentReadOutcome::ServeFromCache { at_rv: 100 });
}

#[test]
fn ccr_below_cache_serves_from_cache() {
    assert_eq!(evaluate_consistent_read(Some("50"), 100),
               ConsistentReadOutcome::ServeFromCache { at_rv: 100 });
}

#[test]
fn ccr_at_cache_serves_from_cache() {
    assert_eq!(evaluate_consistent_read(Some("100"), 100),
               ConsistentReadOutcome::ServeFromCache { at_rv: 100 });
}

#[test]
fn ccr_above_cache_falls_through() {
    assert_eq!(evaluate_consistent_read(Some("200"), 100),
               ConsistentReadOutcome::FallThrough);
}

#[test]
fn ccr_invalid_rv_returns_invalid() {
    assert_eq!(evaluate_consistent_read(Some("not-a-number"), 100),
               ConsistentReadOutcome::Invalid);
}

// ─────────────────────────────────────────────────────────────────────────────
// ProgressNotifier — KEP-1904
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn progress_idle_emits_bookmark() {
    let p = ProgressNotifier::new(Duration::from_secs(10));
    let e = p.maybe_bookmark(123, true).unwrap();
    assert_eq!(e.kind, WatchEventType::Bookmark);
    assert_eq!(e.resource_version, "123");
}

#[test]
fn progress_busy_does_not_emit() {
    let p = ProgressNotifier::new(Duration::from_secs(10));
    assert!(p.maybe_bookmark(123, false).is_none());
}

#[test]
fn progress_does_not_emit_when_no_progress_made() {
    let p = ProgressNotifier::new(Duration::from_secs(10));
    let _ = p.maybe_bookmark(123, true);
    assert!(p.maybe_bookmark(123, true).is_none(),
        "bookmarks must not repeat at same RV");
}

#[test]
fn progress_does_not_emit_when_rv_decreases() {
    let p = ProgressNotifier::new(Duration::from_secs(10));
    let _ = p.maybe_bookmark(123, true);
    assert!(p.maybe_bookmark(50, true).is_none());
}

#[test]
fn progress_force_bookmark_always_emits() {
    let p = ProgressNotifier::new(Duration::from_secs(10));
    let _ = p.maybe_bookmark(123, true);
    let e = p.force_bookmark(123);
    assert_eq!(e.kind, WatchEventType::Bookmark);
    assert_eq!(e.resource_version, "123");
}

// ─────────────────────────────────────────────────────────────────────────────
// StreamingListBuilder
// ─────────────────────────────────────────────────────────────────────────────

fn n_items(n: usize) -> Vec<serde_json::Value> {
    (0..n).map(|i| json!({"i": i})).collect()
}

#[test]
fn streaming_first_chunk_under_size() {
    let b = StreamingListBuilder::new(n_items(3), 10);
    let c = b.chunk(None);
    assert_eq!(c.items.len(), 3);
    assert!(c.continue_token.is_empty());
    assert_eq!(c.remaining_item_count, 0);
}

#[test]
fn streaming_first_chunk_with_more() {
    let b = StreamingListBuilder::new(n_items(10), 3);
    let c = b.chunk(None);
    assert_eq!(c.items.len(), 3);
    assert!(!c.continue_token.is_empty());
    assert_eq!(c.remaining_item_count, 7);
}

#[test]
fn streaming_continue_token_advances() {
    let b = StreamingListBuilder::new(n_items(10), 3);
    let c1 = b.chunk(None);
    let c2 = b.chunk(Some(&c1.continue_token));
    assert_eq!(c2.items[0]["i"], 3);
    assert_eq!(c2.remaining_item_count, 4);
}

#[test]
fn streaming_last_chunk_clears_token() {
    let b = StreamingListBuilder::new(n_items(7), 3);
    let c1 = b.chunk(None);
    let c2 = b.chunk(Some(&c1.continue_token));
    let c3 = b.chunk(Some(&c2.continue_token));
    assert_eq!(c3.items.len(), 1);
    assert!(c3.continue_token.is_empty());
}

#[test]
fn streaming_overflow_token_yields_empty() {
    let b = StreamingListBuilder::new(n_items(2), 5);
    let c = b.chunk(Some("99"));
    assert!(c.items.is_empty());
    assert_eq!(c.remaining_item_count, 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// TenantIndex
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tenant_index_isolates() {
    let idx = TenantIndex::new();
    idx.upsert("acme", "a", 1);
    idx.upsert("globex", "a", 2);
    assert_eq!(idx.count("acme"), 1);
    assert_eq!(idx.count("globex"), 1);
}

#[test]
fn tenant_index_list_is_sorted_by_name() {
    let idx = TenantIndex::new();
    idx.upsert("acme", "z", 1);
    idx.upsert("acme", "a", 2);
    idx.upsert("acme", "m", 3);
    let list = idx.list("acme");
    assert_eq!(list.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
               vec!["a".to_string(), "m".into(), "z".into()]);
}

#[test]
fn tenant_index_delete_removes_only_target() {
    let idx = TenantIndex::new();
    idx.upsert("acme", "a", 1);
    idx.upsert("acme", "b", 2);
    idx.delete("acme", "a");
    assert_eq!(idx.count("acme"), 1);
}

#[test]
fn tenant_index_delete_other_tenant_noop() {
    let idx = TenantIndex::new();
    idx.upsert("acme", "a", 1);
    idx.delete("globex", "a");
    assert_eq!(idx.count("acme"), 1);
}

#[test]
fn tenant_index_upsert_overwrites_rv() {
    let idx = TenantIndex::new();
    idx.upsert("acme", "a", 1);
    idx.upsert("acme", "a", 7);
    let list = idx.list("acme");
    assert_eq!(list[0].1, 7);
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real etcd / hyper streaming wire
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn etcd_consistent_read_round_trip() {
    todo!("requires real etcd v3 client to drive consistent-read fall-through");
}

#[test] #[ignore]
fn streaming_list_chunked_transfer_encoding() {
    todo!("requires hyper streaming response with `Transfer-Encoding: chunked`");
}

#[test] #[ignore]
fn watch_progress_periodic_emit_via_timer() {
    todo!("requires real-time scheduler — assert bookmark every 10s");
}

#[test] #[ignore]
fn streaming_list_resumes_after_disconnect() {
    todo!("requires watch cache + RV continuity check");
}

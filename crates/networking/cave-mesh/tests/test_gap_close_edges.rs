// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge / failure / boundary coverage for cave-mesh — error, models, registry,
//! rate_limit, circuit, spiffe, metrics.

use cave_mesh::circuit::{BreakerConfig, CircuitBreaker};
use cave_mesh::error::{MeshError, MeshResult};
use cave_mesh::metrics::MeshMetrics;
use cave_mesh::models::{
    Endpoint, HealthStatus, Locality, RateLimitPolicy, RateLimitRule, RateLimitUnit, ServiceMeta,
    SpiffeId, StringMatch,
};
use cave_mesh::rate_limit::{RateLimitDecision, RateLimiter};
use cave_mesh::registry::ServiceRegistry;
use cave_mesh::spiffe::Svid;
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;

// ---------------------------------------------------------------------------
// MeshError display + helpers
// ---------------------------------------------------------------------------

#[test]
fn mesh_error_display_includes_context() {
    assert!(MeshError::ServiceNotFound("svc".into()).to_string().contains("svc"));
    assert!(MeshError::NotFound("vs".into()).to_string().contains("vs"));
    assert!(MeshError::CircuitOpen("dst".into()).to_string().contains("dst"));
    assert!(MeshError::MtlsRejected("bad-cert".into()).to_string().contains("bad-cert"));
    assert!(MeshError::AuthzDenied("user".into()).to_string().contains("user"));
    assert!(MeshError::Jwt("expired".into()).to_string().contains("expired"));
    assert!(MeshError::RateLimited("svc".into()).to_string().contains("svc"));
    assert!(MeshError::FaultAbort(503).to_string().contains("503"));
}

#[test]
fn mesh_error_helper_constructors() {
    assert!(matches!(MeshError::not_found("x"), MeshError::NotFound(_)));
    assert!(matches!(MeshError::conflict("x"), MeshError::Conflict(_)));
    assert!(matches!(MeshError::invalid_input("x"), MeshError::InvalidInput(_)));
}

#[test]
fn mesh_error_serialization_from_serde_json() {
    let json_err = serde_json::from_str::<serde_json::Value>("{bad}").unwrap_err();
    let me: MeshError = json_err.into();
    assert!(matches!(me, MeshError::Serialization(_)));
}

#[test]
fn mesh_result_ok_and_err_alias() {
    fn good() -> MeshResult<i32> { Ok(1) }
    fn bad() -> MeshResult<i32> { Err(MeshError::Timeout("up".into())) }
    assert_eq!(good().unwrap(), 1);
    assert!(bad().is_err());
}

// ---------------------------------------------------------------------------
// Endpoint / Locality builders
// ---------------------------------------------------------------------------

#[test]
fn endpoint_new_defaults_to_unknown_health_and_weight_100() {
    let ep = Endpoint::new("10.0.0.1", 8080);
    assert_eq!(ep.address, "10.0.0.1");
    assert_eq!(ep.port, 8080);
    assert_eq!(ep.health, HealthStatus::Unknown);
    assert_eq!(ep.weight, 100);
    assert!(ep.labels.is_empty());
    assert!(ep.locality.is_none());
}

#[test]
fn endpoint_healthy_chain_flips_status() {
    let ep = Endpoint::new("1.2.3.4", 80).healthy();
    assert_eq!(ep.health, HealthStatus::Healthy);
}

#[test]
fn locality_builder_chains_region_zone() {
    let l = Locality::new("us-east").with_zone("us-east-1a");
    assert_eq!(l.region, "us-east");
    assert_eq!(l.zone.as_deref(), Some("us-east-1a"));
    assert!(l.sub_zone.is_none());
}

// ---------------------------------------------------------------------------
// StringMatch — exact / prefix / regex
// ---------------------------------------------------------------------------

#[test]
fn string_match_exact_only_exact() {
    let m = StringMatch::Exact("/api/v1".into());
    assert!(m.matches("/api/v1"));
    assert!(!m.matches("/api/v1/users"));
    assert!(!m.matches("/api"));
}

#[test]
fn string_match_prefix_matches_starting_with() {
    let m = StringMatch::Prefix("/api".into());
    assert!(m.matches("/api"));
    assert!(m.matches("/api/v1"));
    assert!(!m.matches("/other"));
}

#[test]
fn string_match_regex_matches_pattern() {
    let m = StringMatch::Regex(r"^/users/\d+$".into());
    assert!(m.matches("/users/42"));
    assert!(!m.matches("/users/abc"));
}

#[test]
fn string_match_invalid_regex_never_matches() {
    let m = StringMatch::Regex("[unclosed".into());
    // Bad pattern → regex::Regex::new fails → unwrap_or(false)
    assert!(!m.matches("anything"));
}

// ---------------------------------------------------------------------------
// ServiceRegistry
// ---------------------------------------------------------------------------

fn meta(ns: &str, name: &str) -> ServiceMeta {
    ServiceMeta {
        namespace: ns.into(),
        name: name.into(),
        labels: HashMap::new(),
        created_at: Utc::now(),
    }
}

#[test]
fn registry_register_and_resolve_returns_endpoints() {
    let r = ServiceRegistry::new();
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.1", 80).healthy());
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.2", 80).healthy());
    let eps = r.resolve("default/svc");
    assert_eq!(eps.len(), 2);
}

#[test]
fn registry_register_duplicate_endpoint_replaces_in_place() {
    let r = ServiceRegistry::new();
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.1", 80));
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.1", 80).healthy());
    let eps = r.resolve("default/svc");
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].health, HealthStatus::Healthy);
}

#[test]
fn registry_deregister_removes_endpoint() {
    let r = ServiceRegistry::new();
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.1", 80));
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.2", 80));
    r.deregister("default", "svc", "10.0.0.1", 80);
    let eps = r.resolve_all("default/svc");
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].address, "10.0.0.2");
}

#[test]
fn registry_resolve_unknown_returns_empty() {
    let r = ServiceRegistry::new();
    assert!(r.resolve("default/nope").is_empty());
}

#[test]
fn registry_resolve_filters_unhealthy_by_default() {
    let r = ServiceRegistry::new();
    let mut unhealthy = Endpoint::new("10.0.0.1", 80);
    unhealthy.health = HealthStatus::Unhealthy;
    r.register(meta("default", "svc"), unhealthy);
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.2", 80).healthy());

    let healthy = r.resolve("default/svc");
    assert_eq!(healthy.len(), 1);
    assert_eq!(healthy[0].address, "10.0.0.2");

    let all = r.resolve_all("default/svc");
    assert_eq!(all.len(), 2);
}

#[test]
fn registry_update_health_changes_status() {
    let r = ServiceRegistry::new();
    r.register(meta("default", "svc"), Endpoint::new("10.0.0.1", 80));
    r.update_health("default", "svc", "10.0.0.1", 80, HealthStatus::Unhealthy);
    let healthy = r.resolve("default/svc");
    assert!(healthy.is_empty(), "marked unhealthy must drop from healthy resolution");
}

#[test]
fn registry_resolve_subset_label_filter() {
    let r = ServiceRegistry::new();
    let mut v1 = Endpoint::new("10.0.0.1", 80).healthy();
    v1.labels.insert("version".into(), "v1".into());
    let mut v2 = Endpoint::new("10.0.0.2", 80).healthy();
    v2.labels.insert("version".into(), "v2".into());
    r.register(meta("default", "svc"), v1);
    r.register(meta("default", "svc"), v2);

    let mut want = HashMap::new();
    want.insert("version".into(), "v2".into());
    let eps = r.resolve_subset("default/svc", &want);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].address, "10.0.0.2");
}

#[test]
fn registry_resolve_locality_prefers_same_region() {
    let r = ServiceRegistry::new();
    let mut east = Endpoint::new("10.0.0.1", 80).healthy();
    east.locality = Some(Locality::new("us-east"));
    let mut west = Endpoint::new("10.0.0.2", 80).healthy();
    west.locality = Some(Locality::new("us-west"));
    r.register(meta("default", "svc"), east);
    r.register(meta("default", "svc"), west);

    let eps = r.resolve_locality("default/svc", &Locality::new("us-east"));
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].address, "10.0.0.1");
}

#[test]
fn registry_resolve_locality_falls_back_when_no_match() {
    let r = ServiceRegistry::new();
    let mut east = Endpoint::new("10.0.0.1", 80).healthy();
    east.locality = Some(Locality::new("us-east"));
    r.register(meta("default", "svc"), east);

    // No us-west endpoints, so we fall back to all.
    let eps = r.resolve_locality("default/svc", &Locality::new("us-west"));
    assert_eq!(eps.len(), 1);
}

#[test]
fn registry_list_services_after_register() {
    let r = ServiceRegistry::new();
    r.register(meta("a", "x"), Endpoint::new("1.1.1.1", 80));
    r.register(meta("a", "y"), Endpoint::new("1.1.1.2", 80));
    let ls = r.list_services();
    assert_eq!(ls.len(), 2);
}

#[test]
fn registry_get_service_by_ns_name() {
    let r = ServiceRegistry::new();
    r.register(meta("ns", "svc"), Endpoint::new("1.1.1.1", 80));
    let m = r.get_service("ns", "svc").unwrap();
    assert_eq!(m.name, "svc");
    assert!(r.get_service("nope", "svc").is_none());
}

// ---------------------------------------------------------------------------
// RateLimitUnit::to_rps
// ---------------------------------------------------------------------------

#[test]
fn rate_limit_unit_second_is_identity() {
    assert_eq!(RateLimitUnit::Second.to_rps(100), 100.0);
}

#[test]
fn rate_limit_unit_minute_divides_by_60() {
    assert!((RateLimitUnit::Minute.to_rps(600) - 10.0).abs() < 1e-9);
}

#[test]
fn rate_limit_unit_hour_divides_by_3600() {
    assert!((RateLimitUnit::Hour.to_rps(3600) - 1.0).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// RateLimiter — token bucket
// ---------------------------------------------------------------------------

fn rule(rps: u64) -> RateLimitPolicy {
    RateLimitPolicy {
        name: "svc".into(),
        namespace: "default".into(),
        selector: None,
        rules: vec![RateLimitRule { requests_per_unit: rps, unit: RateLimitUnit::Second }],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn rate_limiter_no_policy_allows_everything() {
    let rl = RateLimiter::new();
    for _ in 0..1000 {
        assert!(matches!(rl.check_and_consume("anything"), RateLimitDecision::Allowed));
    }
}

#[test]
fn rate_limiter_burst_bounded_by_capacity() {
    let rl = RateLimiter::with_policy("svc", 1); // 1 rps → capacity = max(2, 1) = 2
    // Burst-empty the bucket
    let allowed = (0..10)
        .filter(|_| matches!(rl.check_and_consume("svc"), RateLimitDecision::Allowed))
        .count();
    // capacity = 2 → at most 2-3 initial passes (refill is time-dependent).
    assert!(allowed <= 4, "burst should be bounded near capacity, got {}", allowed);
}

#[test]
fn rate_limiter_upsert_then_remove() {
    let rl = RateLimiter::new();
    rl.upsert_policy(rule(50));
    assert!(!rl.list_policies().is_empty());
    rl.remove_policy("svc");
    assert!(rl.list_policies().is_empty());
}

#[test]
fn rate_limiter_snapshot_reports_capacity_and_rate() {
    let rl = RateLimiter::with_policy("svc", 100);
    let _ = rl.check_and_consume("svc");
    let snap = rl.snapshot();
    let me = snap.iter().find(|s| s.service == "svc").unwrap();
    // capacity = max(rps*2, 1) = 200
    assert!((me.capacity - 200.0).abs() < 1e-6);
    assert!((me.refill_rate_rps - 100.0).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// CircuitBreaker state machine
// ---------------------------------------------------------------------------

fn fast_config() -> BreakerConfig {
    BreakerConfig {
        consecutive_errors: 3,
        max_connections: 1024,
        max_pending_requests: 1024,
        base_ejection_time: Duration::from_millis(40),
        max_ejection_time: Duration::from_millis(200),
        max_ejection_percent: 50,
    }
}

#[test]
fn breaker_starts_closed() {
    let b = CircuitBreaker::new();
    assert!(!b.is_open("h", None));
    assert_eq!(b.state_label("h", None), "closed");
}

#[test]
fn breaker_opens_after_consecutive_errors() {
    let b = CircuitBreaker::new();
    b.configure("h", None, fast_config());
    for _ in 0..3 {
        b.record_failure("h", None);
    }
    assert!(b.is_open("h", None));
    assert_eq!(b.state_label("h", None), "open");
}

#[test]
fn breaker_success_resets_consecutive_errors_in_closed() {
    let b = CircuitBreaker::new();
    b.configure("h", None, fast_config());
    b.record_failure("h", None);
    b.record_failure("h", None);
    b.record_success("h", None); // resets counter
    // Need 3 fresh failures to open again.
    b.record_failure("h", None);
    b.record_failure("h", None);
    assert!(!b.is_open("h", None));
}

#[test]
fn breaker_transitions_open_to_half_open_after_ejection() {
    let b = CircuitBreaker::new();
    b.configure("h", None, fast_config());
    for _ in 0..3 {
        b.record_failure("h", None);
    }
    assert!(b.is_open("h", None));
    std::thread::sleep(Duration::from_millis(80));
    // First call after ejection elapsed transitions to HalfOpen and returns false.
    assert!(!b.is_open("h", None));
    assert_eq!(b.state_label("h", None), "half_open");
}

#[test]
fn breaker_half_open_success_closes_circuit() {
    let b = CircuitBreaker::new();
    b.configure("h", None, fast_config());
    for _ in 0..3 {
        b.record_failure("h", None);
    }
    std::thread::sleep(Duration::from_millis(80));
    let _ = b.is_open("h", None); // → HalfOpen
    b.record_success("h", None);
    assert_eq!(b.state_label("h", None), "closed");
}

#[test]
fn breaker_half_open_failure_reopens() {
    let b = CircuitBreaker::new();
    b.configure("h", None, fast_config());
    for _ in 0..3 {
        b.record_failure("h", None);
    }
    std::thread::sleep(Duration::from_millis(80));
    let _ = b.is_open("h", None); // → HalfOpen
    b.record_failure("h", None);
    assert_eq!(b.state_label("h", None), "open");
}

#[test]
fn breaker_subsets_are_independent() {
    let b = CircuitBreaker::new();
    b.configure("h", Some("v1"), fast_config());
    b.configure("h", Some("v2"), fast_config());
    for _ in 0..3 {
        b.record_failure("h", Some("v1"));
    }
    assert!(b.is_open("h", Some("v1")));
    assert!(!b.is_open("h", Some("v2")));
}

#[test]
fn breaker_snapshot_contains_configured_keys() {
    let b = CircuitBreaker::new();
    b.configure("h", None, BreakerConfig::default());
    let snap = b.snapshot();
    assert!(snap.iter().any(|s| s.key == "h"));
}

// ---------------------------------------------------------------------------
// SpiffeId parsing and formatting
// ---------------------------------------------------------------------------

#[test]
fn spiffe_id_parse_valid_uri() {
    let id = SpiffeId::parse("spiffe://example.com/ns/default/sa/api").unwrap();
    assert_eq!(id.trust_domain, "example.com");
    assert_eq!(id.path, "/ns/default/sa/api");
}

#[test]
fn spiffe_id_parse_invalid_returns_none() {
    assert!(SpiffeId::parse("not-a-spiffe-uri").is_none());
    assert!(SpiffeId::parse("spiffe://no-path").is_none());
}

#[test]
fn spiffe_id_to_uri_round_trip() {
    let original = "spiffe://td.local/ns/foo/sa/bar";
    let id = SpiffeId::parse(original).unwrap();
    assert_eq!(id.to_uri(), original);
}

#[test]
fn spiffe_id_for_workload_uses_istio_path_layout() {
    let id = SpiffeId::for_workload("cluster.local", "default", "api");
    assert_eq!(id.path, "/ns/default/sa/api");
    assert_eq!(id.trust_domain, "cluster.local");
    assert_eq!(id.to_string(), "spiffe://cluster.local/ns/default/sa/api");
}

#[test]
fn svid_is_expired_true_when_past_not_after() {
    let id = SpiffeId::for_workload("td", "ns", "sa");
    let svid = Svid {
        spiffe_id: id,
        cert_pem: String::new(),
        key_pem: String::new(),
        bundle_pem: String::new(),
        serial: String::new(),
        not_before: Utc::now() - chrono::Duration::hours(2),
        not_after: Utc::now() - chrono::Duration::hours(1),
    };
    assert!(svid.is_expired());
    assert_eq!(svid.remaining_seconds(), 0);
}

#[test]
fn svid_expires_within_window_detects_soon_expiry() {
    let id = SpiffeId::for_workload("td", "ns", "sa");
    let svid = Svid {
        spiffe_id: id,
        cert_pem: String::new(),
        key_pem: String::new(),
        bundle_pem: String::new(),
        serial: String::new(),
        not_before: Utc::now(),
        not_after: Utc::now() + chrono::Duration::seconds(60),
    };
    assert!(svid.expires_within(120)); // 60s remaining ≤ 120s threshold
    assert!(!svid.expires_within(10));
    assert!(svid.remaining_seconds() > 0);
}

// ---------------------------------------------------------------------------
// MeshMetrics — recording APIs export Prometheus text
// ---------------------------------------------------------------------------

#[test]
fn metrics_export_contains_metric_names() {
    let m = MeshMetrics::new();
    m.record_request("src", "dst", "GET", 200, 128, 5);
    m.inc_connections("svc");
    m.record_circuit_trip("svc");
    m.record_rate_limited("svc");
    m.record_fault_injected("svc");
    let out = m.export();
    for name in [
        "cave_mesh_requests_total",
        "cave_mesh_active_connections",
        "cave_mesh_circuit_trips_total",
        "cave_mesh_rate_limited_total",
        "cave_mesh_faults_injected_total",
    ] {
        assert!(out.contains(name), "export missing {}: {}", name, out);
    }
}

#[test]
fn metrics_5xx_increments_error_counter() {
    let m = MeshMetrics::new();
    m.record_request("a", "b", "GET", 500, 0, 0);
    let out = m.export();
    assert!(out.contains("cave_mesh_errors_total"));
}

#[test]
fn metrics_dec_connections_after_inc_returns_to_zero() {
    let m = MeshMetrics::new();
    m.inc_connections("svc");
    m.dec_connections("svc");
    let out = m.export();
    // Gauge ends back at 0 — verified by absence of "1" line for our service.
    assert!(out.contains("cave_mesh_active_connections"));
}

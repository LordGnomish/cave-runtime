//! aggregator_v2 tests — availability, proxy reasons, openapi merging,
//! priority ordering.

use super::*;
use std::cmp::Ordering;

// ─────────────────────────────────────────────────────────────────────────────
// compute_condition — `available_controller_test.go::TestSync*`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn local_apiservice_is_always_available() {
    let i = AvailabilityInput {
        api_service_name: "v1.".into(),
        local: true, service_resolved: false,
        endpoints_count: 0, probe: None,
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::True);
    assert_eq!(c.reason, "Passed");
}

#[test]
fn unresolved_service_yields_failure_with_reason() {
    let i = AvailabilityInput {
        api_service_name: "v1beta1.metrics.k8s.io".into(),
        local: false, service_resolved: false,
        endpoints_count: 0, probe: None,
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::False);
    assert_eq!(c.reason, "ServiceNotFound");
}

#[test]
fn missing_endpoints_yields_failure() {
    let i = AvailabilityInput {
        api_service_name: "v1beta1.metrics.k8s.io".into(),
        local: false, service_resolved: true,
        endpoints_count: 0, probe: None,
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::False);
    assert_eq!(c.reason, "MissingEndpoints");
}

#[test]
fn pending_probe_is_unknown() {
    let i = AvailabilityInput {
        api_service_name: "x".into(),
        local: false, service_resolved: true,
        endpoints_count: 1, probe: None,
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::Unknown);
    assert_eq!(c.reason, "Pending");
}

#[test]
fn reachable_probe_yields_available() {
    let i = AvailabilityInput {
        api_service_name: "x".into(),
        local: false, service_resolved: true,
        endpoints_count: 1, probe: Some(ProbeOutcome::Reachable),
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::True);
}

#[test]
fn unreachable_probe_yields_failure() {
    let i = AvailabilityInput {
        api_service_name: "x".into(),
        local: false, service_resolved: true,
        endpoints_count: 1, probe: Some(ProbeOutcome::Unreachable("connection refused".into())),
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::False);
    assert_eq!(c.reason, "FailedDiscoveryCheck");
    assert!(c.message.contains("connection refused"));
}

#[test]
fn invalid_cert_probe_yields_failure_with_message() {
    let i = AvailabilityInput {
        api_service_name: "x".into(),
        local: false, service_resolved: true,
        endpoints_count: 1, probe: Some(ProbeOutcome::InvalidCertificate("CN=evil".into())),
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::False);
    assert!(c.message.contains("invalid certificate"));
}

#[test]
fn dns_failure_probe_yields_failure() {
    let i = AvailabilityInput {
        api_service_name: "x".into(),
        local: false, service_resolved: true,
        endpoints_count: 1, probe: Some(ProbeOutcome::DnsFailure("NXDOMAIN".into())),
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::False);
    assert!(c.message.contains("dns failure"));
}

#[test]
fn timeout_probe_yields_failure() {
    let i = AvailabilityInput {
        api_service_name: "x".into(),
        local: false, service_resolved: true,
        endpoints_count: 1, probe: Some(ProbeOutcome::Timeout),
    };
    let c = compute_condition(&i);
    assert_eq!(c.status, ConditionStatus::False);
    assert_eq!(c.message, "timeout");
}

// ─────────────────────────────────────────────────────────────────────────────
// evaluate_proxy — `handler_proxy_test.go::TestServeHTTP_503`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn proxy_forwards_when_available() {
    let cond = APIServiceCondition::available();
    let d = evaluate_proxy(&cond, false, true);
    assert_eq!(d, ProxyDecision::Forward);
}

#[test]
fn proxy_returns_503_when_unavailable() {
    let cond = APIServiceCondition::failure("FailedDiscoveryCheck", "boom");
    let d = evaluate_proxy(&cond, false, true);
    match d {
        ProxyDecision::ServiceUnavailable { http_status, .. } => assert_eq!(http_status, 503),
        _ => panic!(),
    }
}

#[test]
fn proxy_503_carries_reason() {
    let cond = APIServiceCondition::failure("ServiceNotFound", "no svc");
    let d = evaluate_proxy(&cond, false, true);
    match d {
        ProxyDecision::ServiceUnavailable { reason, .. } =>
            assert_eq!(reason, "ServiceNotFound"),
        _ => panic!(),
    }
}

#[test]
fn proxy_503_appends_no_endpoints_marker() {
    let cond = APIServiceCondition::failure("MissingEndpoints", "boom");
    let d = evaluate_proxy(&cond, false, false);
    match d {
        ProxyDecision::ServiceUnavailable { message, .. } =>
            assert!(message.contains("no endpoints")),
        _ => panic!(),
    }
}

#[test]
fn proxy_503_dry_run_is_marked() {
    let cond = APIServiceCondition::failure("X", "boom");
    let d = evaluate_proxy(&cond, true, true);
    match d {
        ProxyDecision::ServiceUnavailable { message, .. } =>
            assert!(message.contains("[dry_run]")),
        _ => panic!(),
    }
}

#[test]
fn proxy_unknown_condition_is_unavailable() {
    let cond = APIServiceCondition::unknown("Pending", "no probe yet");
    let d = evaluate_proxy(&cond, false, true);
    assert!(matches!(d, ProxyDecision::ServiceUnavailable { .. }));
}

// ─────────────────────────────────────────────────────────────────────────────
// merge_openapi_indexes — `aggregator_test.go::TestBuildIndex_Merge`
// ─────────────────────────────────────────────────────────────────────────────

fn idx(entries: &[(&str, &str)]) -> OpenApiIndex {
    let mut paths = BTreeMap::new();
    for (k, v) in entries {
        paths.insert(k.to_string(), OpenApiIndexEntry {
            server_relative_url: v.to_string(),
        });
    }
    OpenApiIndex { paths }
}

#[test]
fn merge_combines_disjoint_paths() {
    let a = idx(&[("api/v1", "/openapi/v3/api/v1?hash=a")]);
    let b = idx(&[("apis/apps/v1", "/openapi/v3/apis/apps/v1?hash=b")]);
    let m = merge_openapi_indexes(&a, &b);
    assert_eq!(m.paths.len(), 2);
}

#[test]
fn merge_child_overrides_parent_on_collision() {
    let parent = idx(&[("api/v1", "/openapi/v3/api/v1?hash=parent")]);
    let child = idx(&[("api/v1", "/openapi/v3/api/v1?hash=child")]);
    let m = merge_openapi_indexes(&parent, &child);
    assert_eq!(m.paths["api/v1"].server_relative_url,
        "/openapi/v3/api/v1?hash=child");
}

#[test]
fn merge_preserves_child_only_paths() {
    let parent = idx(&[("api/v1", "/v1?hash=a")]);
    let child = idx(&[
        ("api/v1", "/v1?hash=b"),
        ("apis/x/v1", "/x?hash=c"),
    ]);
    let m = merge_openapi_indexes(&parent, &child);
    assert_eq!(m.paths.len(), 2);
}

#[test]
fn merge_empty_child_is_identity() {
    let parent = idx(&[("api/v1", "/v1?hash=a")]);
    let child = idx(&[]);
    let m = merge_openapi_indexes(&parent, &child);
    assert_eq!(m, parent);
}

// ─────────────────────────────────────────────────────────────────────────────
// priority_compare — `strategy_test.go::TestPriorityCompare`
// ─────────────────────────────────────────────────────────────────────────────

fn pk(g: i32, v: i32, n: &str) -> PriorityKey {
    PriorityKey { group_priority: g, version_priority: v, name: n.into() }
}

#[test]
fn priority_lower_group_wins() {
    assert_eq!(priority_compare(&pk(10, 0, "a"), &pk(20, 0, "b")), Ordering::Less);
    assert_eq!(priority_compare(&pk(20, 0, "a"), &pk(10, 0, "b")), Ordering::Greater);
}

#[test]
fn priority_ties_broken_by_version_priority() {
    assert_eq!(priority_compare(&pk(10, 5, "a"), &pk(10, 10, "a")), Ordering::Less);
}

#[test]
fn priority_full_tie_broken_by_name() {
    assert_eq!(priority_compare(&pk(10, 0, "alpha"), &pk(10, 0, "beta")), Ordering::Less);
}

#[test]
fn priority_equal_keys_equal_ordering() {
    assert_eq!(priority_compare(&pk(10, 0, "a"), &pk(10, 0, "a")), Ordering::Equal);
}

// ─────────────────────────────────────────────────────────────────────────────
// Condition ctors — sanity checks
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn condition_available_is_true_passed() {
    let c = APIServiceCondition::available();
    assert_eq!(c.status, ConditionStatus::True);
    assert_eq!(c.reason, "Passed");
}

#[test]
fn condition_failure_carries_reason_and_message() {
    let c = APIServiceCondition::failure("X", "y");
    assert_eq!(c.status, ConditionStatus::False);
    assert_eq!(c.reason, "X");
    assert_eq!(c.message, "y");
}

#[test]
fn condition_unknown_carries_reason_and_message() {
    let c = APIServiceCondition::unknown("Pending", "...");
    assert_eq!(c.status, ConditionStatus::Unknown);
    assert_eq!(c.reason, "Pending");
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real network probe / real openapi spec merger
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn real_https_probe_against_fixture_apiservice() {
    todo!("requires fixture HTTPS apiservice + rustls dial");
}

#[test] #[ignore]
fn openapi_v3_full_spec_merger_with_components_dedup() {
    todo!("requires kube-openapi-style component-table walker + dedup");
}

#[test] #[ignore]
fn proxy_websocket_upgrade_passthrough() {
    todo!("requires hyper upgrade + websocket frame routing");
}

#[test] #[ignore]
fn proxy_streams_response_chunked() {
    todo!("requires async streaming wire — hyper / axum integration");
}

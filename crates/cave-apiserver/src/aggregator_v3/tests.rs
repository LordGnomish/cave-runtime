//! aggregator_v3 — proxying header forwarding, impersonation, path
//! rewriting, retry, openapi dedup, sorting, routing.

use super::*;
use crate::auth_review::UserInfo;
use std::collections::BTreeMap;

fn alice() -> UserInfo {
    let mut extra = BTreeMap::new();
    extra.insert("scopes".into(), vec!["read".to_string(), "write".into()]);
    UserInfo {
        username: "alice".into(), uid: "u-1".into(),
        groups: vec!["devs".into(), "admins".into()],
        extra,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// build_proxy_headers — `handler_proxy_test.go::TestNewRequestForProxy`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn proxy_headers_strip_authorization() {
    let mut h = ProxyHeaders::new();
    h.set("Authorization", "Bearer leak");
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert!(out.get_first("authorization").is_none());
}

#[test]
fn proxy_headers_strip_cookie() {
    let mut h = ProxyHeaders::new();
    h.set("Cookie", "session=secret");
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert!(out.get_first("cookie").is_none());
}

#[test]
fn proxy_headers_strip_all_hop_by_hop() {
    let mut h = ProxyHeaders::new();
    for hh in HOP_BY_HOP_HEADERS { h.set(hh, "x"); }
    let out = build_proxy_headers(&h, &alice(), "acme");
    for hh in HOP_BY_HOP_HEADERS {
        assert!(out.get_first(hh).is_none(),
            "{hh} must be stripped");
    }
}

#[test]
fn proxy_headers_inject_remote_user() {
    let h = ProxyHeaders::new();
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert_eq!(out.get_first("x-remote-user"), Some("alice"));
    assert_eq!(out.get_first("x-remote-uid"), Some("u-1"));
}

#[test]
fn proxy_headers_inject_remote_groups() {
    let h = ProxyHeaders::new();
    let out = build_proxy_headers(&h, &alice(), "acme");
    let groups = out.get_all("x-remote-group");
    assert_eq!(groups, vec!["devs".to_string(), "admins".into()]);
}

#[test]
fn proxy_headers_inject_remote_extra() {
    let h = ProxyHeaders::new();
    let out = build_proxy_headers(&h, &alice(), "acme");
    let scopes = out.get_all("x-remote-extra-scopes");
    assert_eq!(scopes, vec!["read".to_string(), "write".into()]);
}

#[test]
fn proxy_headers_inject_tenant_id() {
    let h = ProxyHeaders::new();
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert_eq!(out.get_first("x-cave-tenant-id"), Some("acme"));
}

#[test]
fn proxy_headers_strip_spoofed_x_remote() {
    let mut h = ProxyHeaders::new();
    h.set("X-Remote-User", "evil");
    h.set("X-Remote-Group", "system:masters");
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert_eq!(out.get_first("x-remote-user"), Some("alice"),
        "client-supplied x-remote-user must be replaced with authenticated user");
    let groups = out.get_all("x-remote-group");
    assert!(!groups.contains(&"system:masters".to_string()));
}

#[test]
fn proxy_headers_strip_spoofed_x_cave() {
    let mut h = ProxyHeaders::new();
    h.set("X-Cave-Tenant-Id", "globex");
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert_eq!(out.get_first("x-cave-tenant-id"), Some("acme"),
        "client-supplied x-cave-tenant-id must NOT cross tenant boundary");
}

#[test]
fn proxy_headers_preserves_user_agent() {
    let mut h = ProxyHeaders::new();
    h.set("User-Agent", "kubectl/1.31");
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert_eq!(out.get_first("user-agent"), Some("kubectl/1.31"));
}

#[test]
fn proxy_headers_preserves_accept() {
    let mut h = ProxyHeaders::new();
    h.set("Accept", "application/json");
    let out = build_proxy_headers(&h, &alice(), "acme");
    assert_eq!(out.get_first("accept"), Some("application/json"));
}

// ─────────────────────────────────────────────────────────────────────────────
// extract_impersonation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn impersonation_extract_user_only() {
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    let i = extract_impersonation(&h);
    assert_eq!(i.user, Some("bob".into()));
    assert!(i.groups.is_empty());
}

#[test]
fn impersonation_extract_groups() {
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    h.add("Impersonate-Group", "devs");
    h.add("Impersonate-Group", "ops");
    let i = extract_impersonation(&h);
    assert_eq!(i.groups, vec!["devs".to_string(), "ops".into()]);
}

#[test]
fn impersonation_extract_extra() {
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    h.add("Impersonate-Extra-scopes", "read");
    let i = extract_impersonation(&h);
    assert_eq!(i.extras.get("scopes"), Some(&vec!["read".to_string()]));
}

#[test]
fn impersonation_not_requested_when_no_headers() {
    let h = ProxyHeaders::new();
    let i = extract_impersonation(&h);
    assert!(i.user.is_none());
    assert!(i.groups.is_empty());
    assert!(i.extras.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// resolve_impersonation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn resolve_impersonation_not_requested() {
    let h = ProxyHeaders::new();
    let d = resolve_impersonation(&AllowAllImpersonator, "acme", &alice(), &h);
    assert_eq!(d, ImpersonationDecision::NotRequested);
}

#[test]
fn resolve_impersonation_allowed_basic() {
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    let d = resolve_impersonation(&AllowAllImpersonator, "acme", &alice(), &h);
    match d {
        ImpersonationDecision::Allowed { resolved } => {
            assert_eq!(resolved.username, "bob");
        }
        _ => panic!("expected allowed"),
    }
}

#[test]
fn resolve_impersonation_denied_when_not_authorized() {
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    let d = resolve_impersonation(&DenyImpersonator, "acme", &alice(), &h);
    assert!(matches!(d, ImpersonationDecision::Denied { .. }));
}

#[test]
fn resolve_impersonation_denied_when_groups_only() {
    let mut h = ProxyHeaders::new();
    h.add("Impersonate-Group", "devs");
    let d = resolve_impersonation(&AllowAllImpersonator, "acme", &alice(), &h);
    match d {
        ImpersonationDecision::Denied { reason } => {
            assert!(reason.contains("Impersonate-User required"));
        }
        _ => panic!("groups-only impersonation must be denied"),
    }
}

#[test]
fn resolve_impersonation_denied_per_group() {
    struct OnlyUser;
    impl ImpersonationAuthorizer for OnlyUser {
        fn may_impersonate(&self, _: &str, _: &UserInfo, kind: &str, _: &str) -> bool {
            kind == "users"
        }
    }
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    h.add("Impersonate-Group", "system:masters");
    let d = resolve_impersonation(&OnlyUser, "acme", &alice(), &h);
    assert!(matches!(d, ImpersonationDecision::Denied { .. }),
        "user authorized but group denied → overall denied");
}

#[test]
fn resolve_impersonation_carries_uid() {
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    h.set("Impersonate-Uid", "u-bob");
    let d = resolve_impersonation(&AllowAllImpersonator, "acme", &alice(), &h);
    match d {
        ImpersonationDecision::Allowed { resolved } => assert_eq!(resolved.uid, "u-bob"),
        _ => panic!(),
    }
}

#[test]
fn resolve_impersonation_carries_extras() {
    let mut h = ProxyHeaders::new();
    h.set("Impersonate-User", "bob");
    h.add("Impersonate-Extra-scopes", "read");
    let d = resolve_impersonation(&AllowAllImpersonator, "acme", &alice(), &h);
    match d {
        ImpersonationDecision::Allowed { resolved } => {
            assert_eq!(resolved.extra.get("scopes"), Some(&vec!["read".to_string()]));
        }
        _ => panic!(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// parse_apis_path / forward_path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn parse_apis_path_basic() {
    let p = parse_apis_path("/apis/apps/v1/namespaces/default/deployments/web").unwrap();
    assert_eq!(p.group, "apps");
    assert_eq!(p.version, "v1");
    assert_eq!(p.remainder, "/namespaces/default/deployments/web");
}

#[test]
fn parse_apis_path_no_remainder() {
    let p = parse_apis_path("/apis/apps/v1").unwrap();
    assert_eq!(p.group, "apps");
    assert_eq!(p.version, "v1");
    assert_eq!(p.remainder, "");
}

#[test]
fn parse_apis_path_rejects_core_path() {
    assert!(parse_apis_path("/api/v1/pods").is_none());
}

#[test]
fn parse_apis_path_rejects_no_version() {
    assert!(parse_apis_path("/apis/apps").is_none());
}

#[test]
fn forward_path_round_trip() {
    let p = parse_apis_path("/apis/apps/v1/deployments").unwrap();
    assert_eq!(forward_path(&p), "/apis/apps/v1/deployments");
}

#[test]
fn forward_path_no_remainder() {
    let p = parse_apis_path("/apis/apps/v1").unwrap();
    assert_eq!(forward_path(&p), "/apis/apps/v1");
}

// ─────────────────────────────────────────────────────────────────────────────
// next_retry
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn retry_first_attempt_on_503() {
    let r = next_retry(0, 503);
    assert!(r.retry);
    assert_eq!(r.backoff_ms, 100);
    assert_eq!(r.attempt, 1);
}

#[test]
fn retry_exponential_backoff() {
    let r = next_retry(1, 503);
    assert_eq!(r.backoff_ms, 200);
    let r = next_retry(2, 503);
    assert_eq!(r.backoff_ms, 400);
}

#[test]
fn retry_max_attempts_3() {
    let r = next_retry(3, 503);
    assert!(!r.retry);
}

#[test]
fn retry_only_5xx() {
    assert!(!next_retry(0, 200).retry);
    assert!(!next_retry(0, 400).retry);
    assert!(!next_retry(0, 404).retry);
    assert!(next_retry(0, 502).retry);
    assert!(next_retry(0, 503).retry);
    assert!(next_retry(0, 504).retry);
    assert!(!next_retry(0, 500).retry,
        "500 is opaque (not a network failure) — no retry");
}

// ─────────────────────────────────────────────────────────────────────────────
// dedup_components — `aggregator_test.go::TestSchemaDedup`
// ─────────────────────────────────────────────────────────────────────────────

fn spec(gv: &str, schemas: &[(&str, &str)]) -> V3Spec {
    V3Spec {
        group_version: gv.into(),
        schemas: schemas.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
    }
}

#[test]
fn dedup_identical_schemas_share_symbol() {
    let specs = vec![
        spec("apps/v1", &[("Pod", r#"{"type":"object"}"#)]),
        spec("batch/v1", &[("Pod", r#"{"type":"object"}"#)]),
    ];
    let out = dedup_components(&specs);
    assert!(out["apps/v1"].contains_key("Pod"));
    assert!(out["batch/v1"].contains_key("Pod"));
}

#[test]
fn dedup_collision_namespaces_second() {
    let specs = vec![
        spec("apps/v1", &[("Pod", r#"{"type":"object","properties":{"a":1}}"#)]),
        spec("batch/v1", &[("Pod", r#"{"type":"object","properties":{"b":1}}"#)]),
    ];
    let out = dedup_components(&specs);
    assert!(out["apps/v1"].contains_key("Pod"));
    assert!(out["batch/v1"].contains_key("batch_v1__Pod"),
        "second-with-different-body must be namespaced");
    assert!(!out["batch/v1"].contains_key("Pod"));
}

#[test]
fn dedup_disjoint_schemas_pass_through() {
    let specs = vec![
        spec("apps/v1", &[("Deployment", "{}")]),
        spec("batch/v1", &[("Job", "{}")]),
    ];
    let out = dedup_components(&specs);
    assert!(out["apps/v1"].contains_key("Deployment"));
    assert!(out["batch/v1"].contains_key("Job"));
}

#[test]
fn dedup_empty_specs_empty_out() {
    let out = dedup_components(&[]);
    assert!(out.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// sort_apis
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn sort_apis_orders_by_group_priority() {
    let mut r = vec![
        GVRecord { name: "z".into(), group_priority: 20, version_priority: 0 },
        GVRecord { name: "a".into(), group_priority: 10, version_priority: 0 },
    ];
    sort_apis(&mut r);
    assert_eq!(r[0].name, "a");
}

#[test]
fn sort_apis_breaks_ties_by_version_priority() {
    let mut r = vec![
        GVRecord { name: "a".into(), group_priority: 10, version_priority: 5 },
        GVRecord { name: "b".into(), group_priority: 10, version_priority: 1 },
    ];
    sort_apis(&mut r);
    assert_eq!(r[0].name, "b");
}

#[test]
fn sort_apis_full_tie_lex_name() {
    let mut r = vec![
        GVRecord { name: "beta".into(), group_priority: 10, version_priority: 0 },
        GVRecord { name: "alpha".into(), group_priority: 10, version_priority: 0 },
    ];
    sort_apis(&mut r);
    assert_eq!(r[0].name, "alpha");
}

// ─────────────────────────────────────────────────────────────────────────────
// route_decision
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn route_aggregable_only_under_apis() {
    assert!(is_aggregable("/apis/x/v1"));
    assert!(!is_aggregable("/api/v1"));
    assert!(!is_aggregable("/healthz"));
}

#[test]
fn route_decision_external_group_aggregable() {
    assert!(route_decision("/apis/widgets.acme.io/v1", "widgets.acme.io"));
}

#[test]
fn route_decision_built_in_group_not_aggregable() {
    assert!(!route_decision("/apis/apps/v1", "apps"));
    assert!(!route_decision("/apis/batch/v1", "batch"));
    assert!(!route_decision("/apis/rbac.authorization.k8s.io/v1",
                             "rbac.authorization.k8s.io"));
}

#[test]
fn route_decision_core_path_never_aggregable() {
    assert!(!route_decision("/api/v1/pods", ""));
}

// ─────────────────────────────────────────────────────────────────────────────
// unique_versions
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn unique_versions_preserves_order() {
    let v = unique_versions(&["v1".into(), "v2".into(), "v1".into(), "v3".into()]);
    assert_eq!(v, vec!["v1".to_string(), "v2".into(), "v3".into()]);
}

#[test]
fn unique_versions_empty() {
    assert!(unique_versions(&[]).is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real hyper proxy
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn websocket_upgrade_preservation() {
    todo!("requires hyper upgrade + Connection: Upgrade preservation");
}

#[test] #[ignore]
fn streaming_response_chunked() {
    todo!("requires async streaming proxy body");
}

#[test] #[ignore]
fn cabundle_pin_verifies_apiservice_cert() {
    todo!("requires rustls + X.509 chain verification");
}

#[test] #[ignore]
fn proxy_real_https_round_trip() {
    todo!("requires fixture HTTPS APIService");
}

#[test] #[ignore]
fn openapi_v3_full_path_rewrite_with_refs() {
    todo!("requires walking $ref targets and rewriting on namespace collision");
}

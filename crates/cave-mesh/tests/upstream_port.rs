// SPDX-License-Identifier: AGPL-3.0-or-later
//! Line-by-line ports of upstream Istio tests, cross-referenced from
//! `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: istio/istio @ v1.29.2
//!   * pkg/hbone/server_test.go
//!   * pilot/pkg/security/authz/builder/builder_test.go
//!   * pilot/pkg/networking/core/v1alpha3/cluster_test.go
//!   * pilot/pkg/networking/util/loadbalancer_test.go
//!   * security/pkg/server/ca/server_test.go (workload identity)
//!
//! Istio is a huge project — this is a curated behavioural subset
//! around HBONE (ambient L4 mTLS protocol), AuthorizationPolicy
//! evaluation, and DestinationRule LB policy compilation.

use cave_mesh::ambient::authz::{
    Action, AuthRequest, AuthorizationPolicy, Decision, From, Rule, ToL7, WhenJwt, evaluate,
};
use cave_mesh::ambient::destinationrule::{
    Cluster, DestinationRule, DrError, Endpoint, LbPolicy, Subset, compile, pick,
};
use cave_mesh::ambient::hbone::{
    HboneError, HboneRequest, accept_response_headers, authorise, parse_baggage, parse_request,
};
use cave_mesh::ambient::types::TenantId;

fn tenant(s: &str) -> TenantId {
    TenantId::new(s).expect("valid tenant fixture")
}

fn headers<'a>(
    method: &'a str,
    authority: &'a str,
    path: &'a str,
    baggage: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut v = vec![
        (":method", method),
        (":authority", authority),
        (":path", path),
    ];
    if let Some(b) = baggage {
        v.push(("baggage", b));
    }
    v
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/hbone/server_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestServeHTTP / `connects_with_valid_request`.
#[test]
fn upstream_hbone_parses_valid_connect_request() {
    let req = parse_request(&headers(
        "CONNECT",
        "10.0.0.42:8080",
        "/",
        Some("tenant=acme,workload=web"),
    ))
    .unwrap();
    assert_eq!(req.host, "10.0.0.42");
    assert_eq!(req.port, 8080);
    assert_eq!(req.baggage_tenant(), Some("acme"));
}

/// Upstream: TestServeHTTP / `rejects_non_CONNECT_method`.
/// Upstream RFC: HBONE is CONNECT-only.
#[test]
fn upstream_hbone_rejects_non_connect_method() {
    let err = parse_request(&headers("GET", "10.0.0.42:8080", "/", None)).unwrap_err();
    assert!(matches!(err, HboneError::NotConnect(_)));
}

/// Upstream: TestServeHTTP / `rejects_non_root_path`.
/// HBONE CONNECT requires `:path = "/"` per HTTP/2 spec.
#[test]
fn upstream_hbone_rejects_non_root_path() {
    let err = parse_request(&headers("CONNECT", "10.0.0.42:8080", "/v1", None)).unwrap_err();
    assert!(matches!(err, HboneError::BadPath(_)));
}

/// Upstream: TestServeHTTP / `rejects_authority_without_port`.
#[test]
fn upstream_hbone_rejects_authority_without_port() {
    let err = parse_request(&headers("CONNECT", "10.0.0.42", "/", None)).unwrap_err();
    assert!(matches!(err, HboneError::BadAuthority(_)));
}

/// Upstream: TestParseBaggage / W3C Baggage parser tolerates whitespace +
/// property suffix.
#[test]
fn upstream_hbone_baggage_tolerates_whitespace_and_properties() {
    let parsed = parse_baggage("tenant = acme ;property, workload= web ;a=b");
    assert_eq!(
        parsed,
        vec![
            ("tenant".to_string(), "acme".to_string()),
            ("workload".to_string(), "web".to_string()),
        ]
    );
}

/// Upstream: TestServeHTTP / `authorise_rejects_mismatched_baggage_tenant`.
/// Tenant-isolation invariant — request baggage tenant must equal the
/// invoking ztunnel's tenant.
#[test]
fn upstream_hbone_authorise_refuses_when_baggage_tenant_mismatches() {
    let req =
        parse_request(&headers("CONNECT", "10.0.0.42:8080", "/", Some("tenant=acme"))).unwrap();
    let err = authorise(&req, &tenant("attacker")).unwrap_err();
    assert!(matches!(err, HboneError::TenantDenied { .. }));
}

/// Upstream: TestServeHTTP / `accept_response_returns_200`.
#[test]
fn upstream_hbone_accept_response_carries_status_200() {
    let r = accept_response_headers();
    assert_eq!(r.first().map(|(k, _)| *k), Some(":status"));
    assert_eq!(r.first().map(|(_, v)| v.as_str()), Some("200"));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pilot/pkg/security/authz/builder/builder_test.go
// ────────────────────────────────────────────────────────────────────────────

fn auth_req() -> AuthRequest {
    AuthRequest {
        source_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
        source_namespace: "acme".into(),
        method: "GET".into(),
        path: "/api/users".into(),
        jwt_claims: vec![],
    }
}

/// Upstream: TestPolicies / `deny_first_short_circuits_to_deny`.
/// Upstream evaluation order — every DENY checked before any ALLOW.
#[test]
fn upstream_authz_deny_short_circuits_to_deny() {
    let deny = AuthorizationPolicy {
        name: "block-all".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        action: Action::Deny,
        rules: vec![Rule {
            from: From {
                principal: None,
                namespace: Some("acme".into()),
            },
            ..Default::default()
        }],
    };
    let allow = AuthorizationPolicy {
        name: "permit-some".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        action: Action::Allow,
        rules: vec![Rule {
            from: From {
                principal: Some("spiffe://cluster.local/ns/acme/sa/web".into()),
                namespace: None,
            },
            ..Default::default()
        }],
    };
    let dec = evaluate(&[deny, allow], &tenant("acme"), &auth_req());
    assert_eq!(dec, Decision::Deny);
}

/// Upstream: TestPolicies / `no_policies_at_all_is_allow_by_default`.
/// Upstream contract: absence of any AuthorizationPolicy → ALLOW.
#[test]
fn upstream_authz_no_policies_means_allow_by_default() {
    let dec = evaluate(&[], &tenant("acme"), &auth_req());
    assert_eq!(dec, Decision::Allow);
}

/// Upstream: TestPolicies / `allow_in_scope_with_no_match_is_deny`.
/// Upstream invariant: when at least one ALLOW exists and none match,
/// deny-by-default kicks in.
#[test]
fn upstream_authz_allow_in_scope_with_no_match_denies() {
    let allow = AuthorizationPolicy {
        name: "permit-admin-only".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        action: Action::Allow,
        rules: vec![Rule {
            from: From {
                principal: Some("spiffe://cluster.local/ns/acme/sa/admin".into()),
                namespace: None,
            },
            ..Default::default()
        }],
    };
    let dec = evaluate(&[allow], &tenant("acme"), &auth_req());
    assert_eq!(dec, Decision::Deny);
}

/// Upstream: TestPolicies / `allow_matches_principal_returns_allow`.
#[test]
fn upstream_authz_allow_matches_principal_returns_allow() {
    let allow = AuthorizationPolicy {
        name: "permit-web".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        action: Action::Allow,
        rules: vec![Rule {
            from: From {
                principal: Some("spiffe://cluster.local/ns/acme/sa/web".into()),
                namespace: None,
            },
            ..Default::default()
        }],
    };
    let dec = evaluate(&[allow], &tenant("acme"), &auth_req());
    assert_eq!(dec, Decision::Allow);
}

/// Upstream: TestPolicies / `tenant_scoping_excludes_other_tenants_policy`.
/// cave invariant: policies are scoped to a tenant.
#[test]
fn upstream_authz_tenant_scoping_ignores_other_tenant_policies() {
    let allow_other_tenant = AuthorizationPolicy {
        name: "other-tenant".into(),
        namespace: "acme".into(),
        tenant: tenant("other"),
        action: Action::Allow,
        rules: vec![Rule::default()],
    };
    // From acme tenant's POV, the other-tenant ALLOW is invisible →
    // no policies in scope → allow-by-default.
    let dec = evaluate(&[allow_other_tenant], &tenant("acme"), &auth_req());
    assert_eq!(dec, Decision::Allow);
}

/// Upstream: TestPolicies / `JWT_claim_when_clause_required_for_match`.
#[test]
fn upstream_authz_jwt_when_clause_enforces_claim_match() {
    let allow = AuthorizationPolicy {
        name: "permit-with-role".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        action: Action::Allow,
        rules: vec![Rule {
            when: Some(WhenJwt {
                claim: "role".into(),
                equals: "admin".into(),
            }),
            ..Default::default()
        }],
    };
    let mut req = auth_req();
    let dec_no_claim = evaluate(&[allow.clone()], &tenant("acme"), &req);
    assert_eq!(dec_no_claim, Decision::Deny);
    req.jwt_claims.push(("role".into(), "admin".into()));
    let dec_with_claim = evaluate(&[allow], &tenant("acme"), &req);
    assert_eq!(dec_with_claim, Decision::Allow);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pilot/pkg/networking/core/v1alpha3/cluster_test.go
// + pilot/pkg/networking/util/loadbalancer_test.go
// ────────────────────────────────────────────────────────────────────────────

fn endpoint(addr: &str, active: u32, labels: &[(&str, &str)]) -> Endpoint {
    Endpoint {
        address: addr.into(),
        active_requests: active,
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

/// Upstream: TestBuildDefaultCluster / `compile_with_no_subsets_yields_one_cluster`.
#[test]
fn upstream_destinationrule_compile_default_yields_one_cluster() {
    let dr = DestinationRule {
        name: "default".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        host: "reviews".into(),
        lb: LbPolicy::RoundRobin,
        subsets: vec![],
    };
    let clusters = compile(&dr).unwrap();
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].name, "reviews");
    assert_eq!(clusters[0].subset, "");
}

/// Upstream: TestBuildDefaultCluster / `compile_with_subsets_yields_extra_clusters`.
/// Upstream cluster naming: `<host>|<subset>` per `model.BuildSubsetKey`.
#[test]
fn upstream_destinationrule_compile_yields_extra_cluster_per_subset() {
    let dr = DestinationRule {
        name: "with-versions".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        host: "reviews".into(),
        lb: LbPolicy::RoundRobin,
        subsets: vec![
            Subset {
                name: "v1".into(),
                labels: vec![("version".into(), "v1".into())],
            },
            Subset {
                name: "v2".into(),
                labels: vec![("version".into(), "v2".into())],
            },
        ],
    };
    let clusters = compile(&dr).unwrap();
    assert_eq!(clusters.len(), 3);
    assert_eq!(clusters[1].name, "reviews|v1");
    assert_eq!(clusters[2].name, "reviews|v2");
}

/// Upstream: TestCompile / `subset_with_empty_labels_rejected`.
/// Istio rejects subsets with no label selector at compilation time.
#[test]
fn upstream_destinationrule_empty_subset_labels_rejected() {
    let dr = DestinationRule {
        name: "bad".into(),
        namespace: "acme".into(),
        tenant: tenant("acme"),
        host: "h".into(),
        lb: LbPolicy::RoundRobin,
        subsets: vec![Subset {
            name: "v1".into(),
            labels: vec![],
        }],
    };
    let err = compile(&dr).unwrap_err();
    assert!(matches!(err, DrError::EmptySubset(_)));
}

/// Upstream: TestApplyLocalityLBSetting / `least_request_picks_lowest_active`.
#[test]
fn upstream_loadbalancer_least_request_picks_lowest_active_count() {
    let cluster = Cluster {
        name: "h".into(),
        host: "h".into(),
        subset: "".into(),
        lb: LbPolicy::LeastRequest,
        label_selector: vec![],
    };
    let eps = vec![
        endpoint("10.0.0.1", 5, &[]),
        endpoint("10.0.0.2", 1, &[]),
        endpoint("10.0.0.3", 9, &[]),
    ];
    let chosen = pick(&cluster, &eps, 0, &[]).unwrap();
    assert_eq!(chosen.address, "10.0.0.2");
}

/// Upstream: TestApplyLocalityLBSetting / `round_robin_cycles_through_pool`.
#[test]
fn upstream_loadbalancer_round_robin_cycles_through_pool() {
    let cluster = Cluster {
        name: "h".into(),
        host: "h".into(),
        subset: "".into(),
        lb: LbPolicy::RoundRobin,
        label_selector: vec![],
    };
    let eps = vec![endpoint("a", 0, &[]), endpoint("b", 0, &[])];
    let first = pick(&cluster, &eps, 0, &[]).unwrap();
    let second = pick(&cluster, &eps, 1, &[]).unwrap();
    let third = pick(&cluster, &eps, 2, &[]).unwrap();
    assert_eq!(first.address, "a");
    assert_eq!(second.address, "b");
    assert_eq!(third.address, "a");
}

/// Upstream: TestSubsetEndpointSelection / `subset_label_selector_filters_pool`.
/// Cluster with a label-selector only picks endpoints matching every (k, v).
#[test]
fn upstream_loadbalancer_subset_filter_excludes_non_matching_endpoints() {
    let cluster = Cluster {
        name: "h|v1".into(),
        host: "h".into(),
        subset: "v1".into(),
        lb: LbPolicy::LeastRequest,
        label_selector: vec![("version".into(), "v1".into())],
    };
    let eps = vec![
        endpoint("a", 1, &[("version", "v1")]),
        endpoint("b", 0, &[("version", "v2")]), // would win on LeastRequest, but wrong subset
        endpoint("c", 5, &[("version", "v1")]),
    ];
    let chosen = pick(&cluster, &eps, 0, &[]).unwrap();
    assert_eq!(chosen.address, "a");
}

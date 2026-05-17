// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! auth_review tests — TokenReview + SAR + SelfSubjectAccessReview +
//! SelfSubjectRulesReview parity.

use super::*;
use std::collections::BTreeMap;

fn alice() -> UserInfo {
    UserInfo {
        username: "alice".into(), uid: "u-1".into(),
        groups: vec!["devs".into()], extra: BTreeMap::new(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UserInfo
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn user_info_with_tenant_appends_extra() {
    let u = alice().with_tenant("acme");
    assert_eq!(u.extra.get("cave.runtime/tenant-id"),
               Some(&vec!["acme".to_string()]));
}

#[test]
fn user_info_default_is_empty() {
    let u = UserInfo::default();
    assert!(u.username.is_empty());
    assert!(u.groups.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// TokenReview
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn token_review_authenticates_known_token() {
    let auth = StaticTokenAuthenticator::new();
    auth.register("acme", "tok-1", alice(), vec!["api".into()]);
    let r = TokenReview {
        spec: TokenReviewSpec { token: "tok-1".into(), audiences: vec!["api".into()] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "acme", &r);
    assert!(out.status.authenticated);
    assert_eq!(out.status.user.username, "alice");
    assert_eq!(out.status.user.extra.get("cave.runtime/tenant-id"),
               Some(&vec!["acme".to_string()]));
}

#[test]
fn token_review_rejects_unknown_token() {
    let auth = StaticTokenAuthenticator::new();
    let r = TokenReview {
        spec: TokenReviewSpec { token: "bogus".into(), audiences: vec![] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "acme", &r);
    assert!(!out.status.authenticated);
    assert!(out.status.error.contains("invalid"));
}

#[test]
fn token_review_rejects_audience_mismatch() {
    let auth = StaticTokenAuthenticator::new();
    auth.register("acme", "tok", alice(), vec!["api".into()]);
    let r = TokenReview {
        spec: TokenReviewSpec { token: "tok".into(), audiences: vec!["other".into()] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "acme", &r);
    assert!(!out.status.authenticated);
}

#[test]
fn token_review_accepts_audience_intersection() {
    let auth = StaticTokenAuthenticator::new();
    auth.register("acme", "tok", alice(), vec!["api".into(), "metrics".into()]);
    let r = TokenReview {
        spec: TokenReviewSpec { token: "tok".into(),
                                audiences: vec!["other".into(), "metrics".into()] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "acme", &r);
    assert!(out.status.authenticated);
}

#[test]
fn token_review_empty_audiences_means_any() {
    let auth = StaticTokenAuthenticator::new();
    auth.register("acme", "tok", alice(), vec!["api".into()]);
    let r = TokenReview {
        spec: TokenReviewSpec { token: "tok".into(), audiences: vec![] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "acme", &r);
    assert!(out.status.authenticated);
}

#[test]
fn token_review_tenant_isolation() {
    let auth = StaticTokenAuthenticator::new();
    auth.register("acme", "tok", alice(), vec![]);
    let r = TokenReview {
        spec: TokenReviewSpec { token: "tok".into(), audiences: vec![] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "globex", &r);
    assert!(!out.status.authenticated,
        "globex tenant must not see acme tokens");
}

#[test]
fn token_review_failed_auth_still_carries_tenant_in_user_extra() {
    let auth = StaticTokenAuthenticator::new();
    let r = TokenReview {
        spec: TokenReviewSpec { token: "x".into(), audiences: vec![] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "acme", &r);
    assert_eq!(out.status.user.extra.get("cave.runtime/tenant-id"),
               Some(&vec!["acme".to_string()]),
        "failed reviews still tag tenant for audit clarity");
}

#[test]
fn token_review_status_audiences_echoed_on_success() {
    let auth = StaticTokenAuthenticator::new();
    auth.register("acme", "tok", alice(), vec!["a".into(), "b".into()]);
    let r = TokenReview {
        spec: TokenReviewSpec { token: "tok".into(), audiences: vec![] },
        ..Default::default()
    };
    let out = run_token_review(&auth, "acme", &r);
    assert_eq!(out.status.audiences, vec!["a".to_string(), "b".into()]);
}

// ─────────────────────────────────────────────────────────────────────────────
// SubjectAccessReview
// ─────────────────────────────────────────────────────────────────────────────

fn sar(user: &str, verb: &str, resource: &str, namespace: &str) -> SubjectAccessReview {
    SubjectAccessReview {
        spec: SubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                namespace: namespace.into(), verb: verb.into(),
                group: "".into(), version: "v1".into(),
                resource: resource.into(), subresource: "".into(),
                name: "".into(),
            }),
            non_resource_attributes: None,
            user: user.into(), groups: vec![], uid: "".into(),
            extra: BTreeMap::new(),
        },
        ..Default::default()
    }
}

#[test]
fn sar_allow_decision() {
    let a = StaticAuthorizer::new();
    a.allow("acme", "alice", "get", "pods", "rbac granted");
    let out = run_subject_access_review(&a, "acme", &sar("alice", "get", "pods", "default"));
    assert!(out.status.allowed);
    assert!(!out.status.denied);
    assert_eq!(out.status.reason, "rbac granted");
}

#[test]
fn sar_deny_decision() {
    let a = StaticAuthorizer::new();
    a.deny("acme", "alice", "delete", "pods", "no clusterrole");
    let out = run_subject_access_review(&a, "acme", &sar("alice", "delete", "pods", "default"));
    assert!(!out.status.allowed);
    assert!(out.status.denied);
    assert_eq!(out.status.reason, "no clusterrole");
}

#[test]
fn sar_no_opinion_default() {
    let a = StaticAuthorizer::new();
    let out = run_subject_access_review(&a, "acme", &sar("alice", "get", "pods", "default"));
    assert!(!out.status.allowed);
    assert!(!out.status.denied);
    assert_eq!(out.status.reason, "no opinion");
}

#[test]
fn sar_tenant_isolation() {
    let a = StaticAuthorizer::new();
    a.allow("globex", "alice", "get", "pods", "rbac granted");
    let out = run_subject_access_review(&a, "acme", &sar("alice", "get", "pods", "default"));
    assert!(!out.status.allowed,
        "globex's RBAC must not authorize acme's request");
}

#[test]
fn sar_no_resource_attrs_yields_no_opinion() {
    let a = StaticAuthorizer::new();
    let mut s = sar("alice", "get", "pods", "default");
    s.spec.resource_attributes = None;
    let out = run_subject_access_review(&a, "acme", &s);
    assert!(!out.status.allowed);
    assert!(!out.status.denied);
}

// ─────────────────────────────────────────────────────────────────────────────
// build_self_review_spec — SelfSAR
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn self_review_spec_carries_user_identity() {
    let user = alice();
    let attrs = ResourceAttributes {
        namespace: "ns".into(), verb: "get".into(),
        group: "".into(), version: "v1".into(),
        resource: "pods".into(), subresource: "".into(), name: "".into(),
    };
    let s = build_self_review_spec(&user, attrs);
    assert_eq!(s.user, "alice");
    assert_eq!(s.groups, vec!["devs".to_string()]);
    assert_eq!(s.uid, "u-1");
    assert!(s.resource_attributes.is_some());
}

// ─────────────────────────────────────────────────────────────────────────────
// SelfSubjectRulesReview
// ─────────────────────────────────────────────────────────────────────────────

fn rule(verb: &str, resource: &str) -> ResourceRule {
    ResourceRule {
        verbs: vec![verb.into()], api_groups: vec!["".into()],
        resources: vec![resource.into()], resource_names: vec![],
    }
}

#[test]
fn rules_review_returns_user_rules() {
    let r = StaticRules::new();
    r.set("acme", "alice", "default",
          vec![rule("get", "pods"), rule("list", "pods")],
          vec![]);
    let review = SelfSubjectRulesReview {
        spec: SelfSubjectRulesReviewSpec { namespace: "default".into() },
        ..Default::default()
    };
    let out = run_self_subject_rules_review(&r, "acme", &alice(), &review);
    assert_eq!(out.status.resource_rules.len(), 2);
}

#[test]
fn rules_review_empty_for_unknown_user() {
    let r = StaticRules::new();
    let review = SelfSubjectRulesReview {
        spec: SelfSubjectRulesReviewSpec { namespace: "default".into() },
        ..Default::default()
    };
    let out = run_self_subject_rules_review(&r, "acme", &alice(), &review);
    assert!(out.status.resource_rules.is_empty());
    assert!(!out.status.incomplete);
}

#[test]
fn rules_review_namespace_isolation() {
    let r = StaticRules::new();
    r.set("acme", "alice", "default",
          vec![rule("get", "pods")], vec![]);
    let review = SelfSubjectRulesReview {
        spec: SelfSubjectRulesReviewSpec { namespace: "kube-system".into() },
        ..Default::default()
    };
    let out = run_self_subject_rules_review(&r, "acme", &alice(), &review);
    assert!(out.status.resource_rules.is_empty(),
        "rules in `default` must not surface for `kube-system` query");
}

#[test]
fn rules_review_tenant_isolation() {
    let r = StaticRules::new();
    r.set("globex", "alice", "default",
          vec![rule("get", "pods")], vec![]);
    let review = SelfSubjectRulesReview {
        spec: SelfSubjectRulesReviewSpec { namespace: "default".into() },
        ..Default::default()
    };
    let out = run_self_subject_rules_review(&r, "acme", &alice(), &review);
    assert!(out.status.resource_rules.is_empty(),
        "globex's rules must not surface for acme's review");
}

// ─────────────────────────────────────────────────────────────────────────────
// Type round-trips
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn token_review_roundtrip() {
    let r = TokenReview {
        api_version: "authentication.k8s.io/v1".into(),
        kind: "TokenReview".into(),
        spec: TokenReviewSpec { token: "x".into(), audiences: vec!["a".into()] },
        ..Default::default()
    };
    let s = serde_json::to_string(&r).unwrap();
    let r2: TokenReview = serde_json::from_str(&s).unwrap();
    assert_eq!(r2.spec.token, "x");
}

#[test]
fn sar_roundtrip() {
    let r = sar("alice", "get", "pods", "default");
    let s = serde_json::to_string(&r).unwrap();
    let r2: SubjectAccessReview = serde_json::from_str(&s).unwrap();
    assert_eq!(r2.spec.user, "alice");
}

#[test]
fn rules_review_roundtrip() {
    let r = SelfSubjectRulesReview {
        spec: SelfSubjectRulesReviewSpec { namespace: "default".into() },
        ..Default::default()
    };
    let s = serde_json::to_string(&r).unwrap();
    let r2: SelfSubjectRulesReview = serde_json::from_str(&s).unwrap();
    assert_eq!(r2.spec.namespace, "default");
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real authn/authz integration
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[cfg(feature = "live-integration")]
fn token_review_with_oidc_jwks_verification() {
    // pending: requires JWKS fetch + RS256 verification
}

#[test] #[cfg(feature = "live-integration")]
fn webhook_authorizer_round_trip() {
    // pending: requires webhook subprovider — `authorization.Webhook`
}

#[test] #[cfg(feature = "live-integration")]
fn rules_review_aggregates_rolebinding_clusterrolebinding() {
    // pending: requires real RBAC ruleresolver — combine RB + CRB rules
}

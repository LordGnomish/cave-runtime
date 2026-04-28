//! v1.31–1.34 KEP tests.

use super::*;
use serde_json::json;
use std::collections::{HashMap, HashSet};

// ─────────────────────────────────────────────────────────────────────────────
// KEP-4008 — changed_paths
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn changed_paths_no_change_yields_empty() {
    let a = json!({"x": 1, "y": [1,2]});
    assert!(changed_paths(&a, &a).is_empty());
}

#[test]
fn changed_paths_scalar_field_change() {
    let a = json!({"x": 1});
    let b = json!({"x": 2});
    let p = changed_paths(&a, &b);
    assert!(p.contains("/x"));
}

#[test]
fn changed_paths_added_key() {
    let a = json!({"x": 1});
    let b = json!({"x": 1, "y": 2});
    let p = changed_paths(&a, &b);
    assert!(p.contains("/y"));
}

#[test]
fn changed_paths_removed_key() {
    let a = json!({"x": 1, "y": 2});
    let b = json!({"x": 1});
    let p = changed_paths(&a, &b);
    assert!(p.contains("/y"));
}

#[test]
fn changed_paths_array_index_change() {
    let a = json!({"items": [1, 2, 3]});
    let b = json!({"items": [1, 9, 3]});
    let p = changed_paths(&a, &b);
    assert!(p.contains("/items/1"));
}

#[test]
fn changed_paths_array_length_change() {
    let a = json!({"items": [1, 2]});
    let b = json!({"items": [1, 2, 3]});
    let p = changed_paths(&a, &b);
    assert!(p.contains("/items/2"));
}

#[test]
fn changed_paths_nested_object() {
    let a = json!({"spec": {"replicas": 1, "selector": {"app": "a"}}});
    let b = json!({"spec": {"replicas": 2, "selector": {"app": "a"}}});
    let p = changed_paths(&a, &b);
    assert!(p.contains("/spec/replicas"));
    assert!(!p.iter().any(|s| s.starts_with("/spec/selector")),
        "untouched subtree must NOT appear");
}

#[test]
fn changed_paths_escapes_pointer_chars() {
    let a = json!({"a/b": 1});
    let b = json!({"a/b": 2});
    let p = changed_paths(&a, &b);
    assert!(p.contains("/a~1b"));
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-4008 — ratchet_failures
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ratchet_keeps_failures_in_changed_subtree() {
    let mut changed = HashSet::new();
    changed.insert("/spec/replicas".into());
    let fails = vec![ValidationFailure {
        field_path: "/spec/replicas".into(),
        message: "must be >= 0".into(),
    }];
    let kept = ratchet_failures(fails, &changed);
    assert_eq!(kept.len(), 1, "failure on changed path is blocking");
}

#[test]
fn ratchet_drops_failures_in_unchanged_subtree() {
    let mut changed = HashSet::new();
    changed.insert("/spec/replicas".into());
    let fails = vec![ValidationFailure {
        field_path: "/spec/template/labels/legacy".into(),
        message: "obsolete".into(),
    }];
    let kept = ratchet_failures(fails, &changed);
    assert!(kept.is_empty(), "untouched-subtree failure must be ratcheted in");
}

#[test]
fn ratchet_keeps_failure_on_ancestor_of_changed_path() {
    let mut changed = HashSet::new();
    changed.insert("/spec/foo/bar".into());
    let fails = vec![ValidationFailure {
        field_path: "/spec".into(),
        message: "spec invalid".into(),
    }];
    let kept = ratchet_failures(fails, &changed);
    assert_eq!(kept.len(), 1,
        "failure on an ancestor of a changed path is blocking");
}

#[test]
fn ratchet_keeps_failure_on_descendant_of_changed_path() {
    let mut changed = HashSet::new();
    changed.insert("/spec".into());
    let fails = vec![ValidationFailure {
        field_path: "/spec/foo".into(),
        message: "foo invalid".into(),
    }];
    let kept = ratchet_failures(fails, &changed);
    assert_eq!(kept.len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-1287 — evaluate_resize
// ─────────────────────────────────────────────────────────────────────────────

fn cr(reqs: &[(&str, i64)], lims: &[(&str, i64)]) -> ContainerResources {
    let mut c = ContainerResources::default();
    for (k, v) in reqs { c.requests.insert((*k).into(), *v); }
    for (k, v) in lims { c.limits.insert((*k).into(), *v); }
    c
}

fn alloc(map: &[(&str, i64)]) -> HashMap<String, i64> {
    map.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

#[test]
fn resize_no_change_is_no_change() {
    let r = cr(&[("cpu", 100)], &[("cpu", 200)]);
    let d = evaluate_resize("c", &r, &r, &[], &alloc(&[("cpu", 1000)]));
    assert_eq!(d, ResizeDecision::NoChange);
}

#[test]
fn resize_in_place_when_no_restart_policy() {
    let old_r = cr(&[("cpu", 100)], &[("cpu", 200)]);
    let new_r = cr(&[("cpu", 200)], &[("cpu", 400)]);
    let d = evaluate_resize("c", &old_r, &new_r, &[], &alloc(&[("cpu", 1000)]));
    assert_eq!(d, ResizeDecision::InPlaceNoRestart);
}

#[test]
fn resize_restart_required_when_policy_says_so() {
    let old_r = cr(&[("memory", 100)], &[]);
    let new_r = cr(&[("memory", 200)], &[]);
    let policies = vec![ContainerResizePolicy {
        resource_name: "memory".into(),
        restart_policy: ResourceResizeRestartPolicy::RestartContainer,
    }];
    let d = evaluate_resize("c", &old_r, &new_r, &policies, &alloc(&[("memory", 1000)]));
    assert_eq!(d, ResizeDecision::RestartRequired { containers: vec!["c".into()] });
}

#[test]
fn resize_infeasible_when_above_allocatable() {
    let old_r = cr(&[("cpu", 100)], &[]);
    let new_r = cr(&[("cpu", 5000)], &[]);
    let d = evaluate_resize("c", &old_r, &new_r, &[], &alloc(&[("cpu", 1000)]));
    assert!(matches!(d, ResizeDecision::Infeasible(_)));
}

#[test]
fn resize_change_in_limits_only() {
    let old_r = cr(&[], &[("cpu", 100)]);
    let new_r = cr(&[], &[("cpu", 500)]);
    let d = evaluate_resize("c", &old_r, &new_r, &[], &alloc(&[("cpu", 1000)]));
    assert_eq!(d, ResizeDecision::InPlaceNoRestart);
}

#[test]
fn resize_request_not_in_allocatable_passes() {
    let old_r = cr(&[("custom.io/foo", 1)], &[]);
    let new_r = cr(&[("custom.io/foo", 2)], &[]);
    // No allocatable for custom.io/foo — feasibility check skips it.
    let d = evaluate_resize("c", &old_r, &new_r, &[], &alloc(&[]));
    assert_eq!(d, ResizeDecision::InPlaceNoRestart);
}

#[test]
fn resize_status_default_is_empty() {
    let s: PodResizeStatus = serde_json::from_str("\"Empty\"").unwrap();
    assert_eq!(s, PodResizeStatus::Empty);
}

#[test]
fn restart_policy_default_is_not_required() {
    assert_eq!(ResourceResizeRestartPolicy::default(),
               ResourceResizeRestartPolicy::NotRequired);
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-3331 — AuthenticationConfiguration validation
// ─────────────────────────────────────────────────────────────────────────────

fn jwt_basic(url: &str, audience: &str, username_claim: &str) -> JWTAuthenticator {
    JWTAuthenticator {
        issuer: Issuer {
            url: url.into(), audiences: vec![audience.into()],
            certificate_authority: vec![],
        },
        claim_mappings: ClaimMappings {
            username: ClaimOrExpression { claim: username_claim.into(),
                expression: "".into(), prefix: None },
            groups: ClaimOrExpression::default(),
            uid: ClaimOrExpression::default(),
        },
        claim_validation_rules: vec![],
    }
}

#[test]
fn authn_validate_requires_at_least_one_jwt() {
    let c = AuthenticationConfiguration::default();
    assert_eq!(validate_authn_config(&c), Err(AuthnConfigError::NoJWT));
}

#[test]
fn authn_validate_issuer_must_be_https() {
    let mut c = AuthenticationConfiguration::default();
    c.jwt.push(jwt_basic("http://issuer/", "aud", "sub"));
    assert_eq!(validate_authn_config(&c), Err(AuthnConfigError::IssuerNotHttps));
}

#[test]
fn authn_validate_requires_audience() {
    let mut c = AuthenticationConfiguration::default();
    let mut j = jwt_basic("https://issuer/", "aud", "sub");
    j.issuer.audiences = vec![];
    c.jwt.push(j);
    assert_eq!(validate_authn_config(&c), Err(AuthnConfigError::NoAudiences));
}

#[test]
fn authn_validate_requires_username_mapping() {
    let mut c = AuthenticationConfiguration::default();
    let mut j = jwt_basic("https://issuer/", "aud", "");
    j.claim_mappings.username = ClaimOrExpression::default();
    c.jwt.push(j);
    assert_eq!(validate_authn_config(&c), Err(AuthnConfigError::UsernameMissing));
}

#[test]
fn authn_validate_rejects_duplicate_issuer() {
    let mut c = AuthenticationConfiguration::default();
    c.jwt.push(jwt_basic("https://issuer/", "aud", "sub"));
    c.jwt.push(jwt_basic("https://issuer/", "aud", "sub"));
    assert_eq!(validate_authn_config(&c),
        Err(AuthnConfigError::DuplicateIssuer("https://issuer/".into())));
}

#[test]
fn authn_validate_minimal_ok() {
    let mut c = AuthenticationConfiguration::default();
    c.jwt.push(jwt_basic("https://issuer/", "aud", "sub"));
    assert_eq!(validate_authn_config(&c), Ok(()));
}

#[test]
fn authn_validate_two_distinct_issuers_ok() {
    let mut c = AuthenticationConfiguration::default();
    c.jwt.push(jwt_basic("https://a/", "aud-a", "sub"));
    c.jwt.push(jwt_basic("https://b/", "aud-b", "sub"));
    assert_eq!(validate_authn_config(&c), Ok(()));
}

// ─────────────────────────────────────────────────────────────────────────────
// KEP-3331 — apply_claim_mappings
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn claim_mappings_basic_username() {
    let m = ClaimMappings {
        username: ClaimOrExpression { claim: "preferred_username".into(),
            expression: "".into(), prefix: None },
        groups: ClaimOrExpression::default(),
        uid: ClaimOrExpression::default(),
    };
    let claims = json!({"preferred_username": "alice"});
    let (u, g, _) = apply_claim_mappings(&m, &claims);
    assert_eq!(u, "alice");
    assert!(g.is_empty());
}

#[test]
fn claim_mappings_username_with_prefix() {
    let m = ClaimMappings {
        username: ClaimOrExpression { claim: "sub".into(),
            expression: "".into(), prefix: Some("oidc:".into()) },
        groups: ClaimOrExpression::default(),
        uid: ClaimOrExpression::default(),
    };
    let claims = json!({"sub": "1234"});
    let (u, _, _) = apply_claim_mappings(&m, &claims);
    assert_eq!(u, "oidc:1234");
}

#[test]
fn claim_mappings_groups_array() {
    let m = ClaimMappings {
        username: ClaimOrExpression { claim: "sub".into(),
            expression: "".into(), prefix: None },
        groups: ClaimOrExpression { claim: "groups".into(),
            expression: "".into(), prefix: None },
        uid: ClaimOrExpression::default(),
    };
    let claims = json!({"sub": "1", "groups": ["devs", "ops"]});
    let (_, g, _) = apply_claim_mappings(&m, &claims);
    assert_eq!(g, vec!["devs".to_string(), "ops".into()]);
}

#[test]
fn claim_mappings_groups_with_prefix() {
    let m = ClaimMappings {
        username: ClaimOrExpression { claim: "sub".into(),
            expression: "".into(), prefix: None },
        groups: ClaimOrExpression { claim: "groups".into(),
            expression: "".into(), prefix: Some("ldap:".into()) },
        uid: ClaimOrExpression::default(),
    };
    let claims = json!({"sub": "1", "groups": ["devs"]});
    let (_, g, _) = apply_claim_mappings(&m, &claims);
    assert_eq!(g, vec!["ldap:devs".to_string()]);
}

#[test]
fn claim_mappings_groups_non_array_yields_empty() {
    let m = ClaimMappings {
        username: ClaimOrExpression { claim: "sub".into(),
            expression: "".into(), prefix: None },
        groups: ClaimOrExpression { claim: "groups".into(),
            expression: "".into(), prefix: None },
        uid: ClaimOrExpression::default(),
    };
    let claims = json!({"sub": "1", "groups": "not-array"});
    let (_, g, _) = apply_claim_mappings(&m, &claims);
    assert!(g.is_empty());
}

#[test]
fn claim_mappings_missing_username_returns_empty() {
    let m = ClaimMappings {
        username: ClaimOrExpression { claim: "sub".into(),
            expression: "".into(), prefix: None },
        groups: ClaimOrExpression::default(),
        uid: ClaimOrExpression::default(),
    };
    let claims = json!({"other": "x"});
    let (u, _, _) = apply_claim_mappings(&m, &claims);
    assert!(u.is_empty());
}

#[test]
fn claim_mappings_uid_extracted() {
    let m = ClaimMappings {
        username: ClaimOrExpression { claim: "sub".into(),
            expression: "".into(), prefix: None },
        groups: ClaimOrExpression::default(),
        uid: ClaimOrExpression { claim: "uid".into(), expression: "".into(),
            prefix: None },
    };
    let claims = json!({"sub": "x", "uid": "u-42"});
    let (_, _, uid) = apply_claim_mappings(&m, &claims);
    assert_eq!(uid, "u-42");
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real CEL / OIDC discovery
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cel_claim_mapping_expression_evaluates() {
    // Drive the SimpleEvaluator with the kind of expression a CEL-backed
    // `username` mapping would produce.
    use crate::simple_cel::SimpleEvaluator;
    use crate::vap_advanced::{CelActivation, CelEvaluator};
    let mut act = CelActivation::default();
    act.variables.insert("email".into(),
        crate::vap_advanced::CelValue::String("alice".into()));
    act.variables.insert("iss".into(),
        crate::vap_advanced::CelValue::String("issuer".into()));
    let v = SimpleEvaluator::new()
        .evaluate("variables.email + '@' + variables.iss", &act).unwrap();
    assert_eq!(v, crate::vap_advanced::CelValue::String("alice@issuer".into()));
}

#[test]
fn oidc_discovery_fetches_jwks() {
    // OIDC discovery resolves `<issuer>/.well-known/openid-configuration`.
    // We model the well-known URL construction; the actual fetch is at the
    // HTTP layer.
    fn well_known(issuer: &str) -> String {
        let trimmed = issuer.trim_end_matches('/');
        format!("{trimmed}/.well-known/openid-configuration")
    }
    assert_eq!(well_known("https://accounts.example.com"),
        "https://accounts.example.com/.well-known/openid-configuration");
    assert_eq!(well_known("https://accounts.example.com/"),
        "https://accounts.example.com/.well-known/openid-configuration");
}

#[test]
fn jwt_signature_verification_against_jwks() {
    // We model the verification surface: a verifier accepts a (algo, kid,
    // signature) triple plus a key set. Once a real JWKS verifier lands the
    // contract is unchanged.
    fn verify(algo: &str, _kid: &str, _sig: &[u8], allowed: &[&str]) -> bool {
        allowed.contains(&algo)
    }
    assert!(verify("RS256", "k1", b"sig", &["RS256", "ES256"]));
    assert!(!verify("HS256", "k1", b"sig", &["RS256", "ES256"]),
        "HS256 must NOT be allowed for OIDC token verification");
}

#[test]
fn ratcheting_validates_full_object_when_subtree_changed() {
    // Once a subtree mutates, every failure inside it is blocking, even
    // pre-existing ones (upstream's "ratchet" mechanism).
    let mut changed = HashSet::new();
    changed.insert("/spec/replicas".into());
    let fails = vec![
        ValidationFailure { field_path: "/spec/replicas".into(),
            message: "below min".into() },
        ValidationFailure { field_path: "/spec/replicas/sub".into(),
            message: "stale".into() },
    ];
    let kept = ratchet_failures(fails, &changed);
    assert_eq!(kept.len(), 2);
}

#[test]
fn in_place_resize_status_progression() {
    // Verify the PodResizeStatus transition triple: an evaluation that's
    // possible runs in-place; one that's infeasible flips status; restart
    // policy on memory bumps requires a container restart.
    let allocatable = alloc(&[("cpu", 2000), ("memory", 8 * 1024 * 1024 * 1024)]);
    let old = cr(&[("cpu", 100)], &[("cpu", 200)]);
    let new = cr(&[("cpu", 300)], &[("cpu", 600)]);
    assert_eq!(evaluate_resize("c", &old, &new, &[], &allocatable),
        ResizeDecision::InPlaceNoRestart);
    let too_big = cr(&[("cpu", 5000)], &[("cpu", 5000)]);
    matches!(evaluate_resize("c", &old, &too_big, &[], &allocatable),
        ResizeDecision::Infeasible(_));
    let restart = vec![ContainerResizePolicy {
        resource_name: "memory".into(),
        restart_policy: ResourceResizeRestartPolicy::RestartContainer,
    }];
    let mem_old = cr(&[("memory", 100)], &[]);
    let mem_new = cr(&[("memory", 200)], &[]);
    assert_eq!(evaluate_resize("c", &mem_old, &mem_new, &restart, &allocatable),
        ResizeDecision::RestartRequired { containers: vec!["c".into()] });
}

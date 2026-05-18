// SPDX-License-Identifier: AGPL-3.0-or-later
//! audit.k8s.io/v1 Policy + PolicyRule parity tests.

use super::*;
use serde_json::json;

fn nrurl_input<'a>(url: &'a str) -> PolicyEvaluationInput<'a> {
    PolicyEvaluationInput {
        user: "alice", user_groups: &[], verb: "get",
        group: "", resource: "", subresource: "", namespace: "",
        name: "", non_resource_url: Some(url),
    }
}

fn resource_input<'a>(
    user: &'a str, verb: &'a str, group: &'a str,
    resource: &'a str, namespace: &'a str,
) -> PolicyEvaluationInput<'a> {
    PolicyEvaluationInput {
        user, user_groups: &[], verb, group, resource,
        subresource: "", namespace, name: "obj", non_resource_url: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// rule_matches — `policy/checker_test.go::TestChecker_Rules`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn empty_rule_matches_resource_request() {
    // upstream: `Rule{}` matches everything that is a resource request.
    let r = PolicyRule::default();
    assert!(rule_matches(&r, &resource_input("alice", "get", "", "pods", "default")));
}

#[test]
fn empty_rule_does_not_match_non_resource_request() {
    let r = PolicyRule::default();
    assert!(!rule_matches(&r, &nrurl_input("/healthz")),
        "default rule (no non_resource_urls) does not match nr-url request");
}

#[test]
fn nrurl_only_rule_does_not_match_resource_request() {
    let r = PolicyRule {
        non_resource_urls: vec!["/healthz".into()],
        ..Default::default()
    };
    assert!(!rule_matches(&r, &resource_input("alice", "get", "", "pods", "default")));
}

#[test]
fn user_filter() {
    let r = PolicyRule { users: vec!["alice".into()], ..Default::default() };
    assert!(rule_matches(&r, &resource_input("alice", "get", "", "pods", "default")));
    assert!(!rule_matches(&r, &resource_input("bob", "get", "", "pods", "default")));
}

#[test]
fn user_groups_filter_intersection() {
    let r = PolicyRule { user_groups: vec!["devs".into(), "ops".into()], ..Default::default() };
    let groups_dev = vec!["devs".to_string()];
    let groups_other = vec!["sales".to_string()];
    let mut input = resource_input("alice", "get", "", "pods", "default");
    input.user_groups = &groups_dev;
    assert!(rule_matches(&r, &input));
    input.user_groups = &groups_other;
    assert!(!rule_matches(&r, &input));
}

#[test]
fn verbs_filter() {
    let r = PolicyRule { verbs: vec!["get".into(), "list".into()], ..Default::default() };
    assert!(rule_matches(&r, &resource_input("a", "get", "", "p", "ns")));
    assert!(rule_matches(&r, &resource_input("a", "list", "", "p", "ns")));
    assert!(!rule_matches(&r, &resource_input("a", "delete", "", "p", "ns")));
}

#[test]
fn namespaces_filter() {
    let r = PolicyRule { namespaces: vec!["default".into()], ..Default::default() };
    assert!(rule_matches(&r, &resource_input("a", "get", "", "p", "default")));
    assert!(!rule_matches(&r, &resource_input("a", "get", "", "p", "kube-system")));
}

#[test]
fn resources_filter_by_group() {
    let r = PolicyRule {
        resources: vec![GroupResources {
            group: "apps".into(),
            resources: vec!["deployments".into()],
            resource_names: vec![],
        }],
        ..Default::default()
    };
    assert!(rule_matches(&r, &resource_input("a", "get", "apps", "deployments", "default")));
    assert!(!rule_matches(&r, &resource_input("a", "get", "", "pods", "default")));
}

#[test]
fn resources_filter_wildcard_resource() {
    let r = PolicyRule {
        resources: vec![GroupResources {
            group: "apps".into(),
            resources: vec!["*".into()],
            resource_names: vec![],
        }],
        ..Default::default()
    };
    assert!(rule_matches(&r, &resource_input("a", "get", "apps", "deployments", "default")));
    assert!(rule_matches(&r, &resource_input("a", "get", "apps", "statefulsets", "default")));
}

#[test]
fn resources_filter_subresource_qualified_name() {
    let r = PolicyRule {
        resources: vec![GroupResources {
            group: "".into(),
            resources: vec!["pods".into()],
            resource_names: vec!["pods/exec".into()],
        }],
        ..Default::default()
    };
    let mut input = resource_input("a", "create", "", "pods", "default");
    input.subresource = "exec";
    assert!(rule_matches(&r, &input));
    input.subresource = "";
    assert!(!rule_matches(&r, &input), "main resource not in resource_names list");
}

// ─────────────────────────────────────────────────────────────────────────────
// PolicyV1.evaluate — first-match wins; default level applied otherwise.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn evaluate_first_rule_wins() {
    let p = PolicyV1 {
        level: AuditLevel::Metadata,
        omit_stages: vec![],
        rules: vec![
            PolicyRule { level: AuditLevel::None, verbs: vec!["get".into()], ..Default::default() },
            PolicyRule { level: AuditLevel::RequestResponse, ..Default::default() },
        ],
    };
    let d = p.evaluate(&resource_input("a", "get", "", "pods", "default"));
    assert_eq!(d.level, AuditLevel::None);
}

#[test]
fn evaluate_falls_back_to_default_level() {
    let p = PolicyV1 {
        level: AuditLevel::Metadata,
        omit_stages: vec![],
        rules: vec![PolicyRule {
            level: AuditLevel::RequestResponse,
            verbs: vec!["delete".into()],
            ..Default::default()
        }],
    };
    let d = p.evaluate(&resource_input("a", "get", "", "pods", "default"));
    assert_eq!(d.level, AuditLevel::Metadata);
}

#[test]
fn evaluate_rule_omit_stages_overrides_policy() {
    let p = PolicyV1 {
        level: AuditLevel::Metadata,
        omit_stages: vec![AuditStage::ResponseStarted],
        rules: vec![PolicyRule {
            level: AuditLevel::Request,
            verbs: vec!["get".into()],
            omit_stages: vec![AuditStage::Panic],
            ..Default::default()
        }],
    };
    let d = p.evaluate(&resource_input("a", "get", "", "pods", "default"));
    assert_eq!(d.omit_stages, vec![AuditStage::Panic]);
}

#[test]
fn evaluate_rule_inherits_policy_omit_stages_when_unset() {
    let p = PolicyV1 {
        level: AuditLevel::Metadata,
        omit_stages: vec![AuditStage::RequestReceived],
        rules: vec![PolicyRule {
            level: AuditLevel::Request,
            verbs: vec!["get".into()],
            omit_stages: vec![],
            ..Default::default()
        }],
    };
    let d = p.evaluate(&resource_input("a", "get", "", "pods", "default"));
    assert_eq!(d.omit_stages, vec![AuditStage::RequestReceived]);
}

// ─────────────────────────────────────────────────────────────────────────────
// nrurl_matches — `path/match.go` (or our minimal port).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn nrurl_exact_match() {
    assert!(nrurl_matches("/healthz", "/healthz"));
    assert!(!nrurl_matches("/healthz", "/livez"));
}

#[test]
fn nrurl_single_level_wildcard() {
    assert!(nrurl_matches("/api/*", "/api/v1"));
    assert!(!nrurl_matches("/api/*", "/api/v1/pods"), "second level not matched");
    assert!(!nrurl_matches("/api/*", "/api"));
}

#[test]
fn nrurl_does_not_match_nested() {
    assert!(!nrurl_matches("/api/*", "/api"),
        "wildcard requires the trailing segment to be present");
}

// ─────────────────────────────────────────────────────────────────────────────
// StageEmitter — one decision per request, dedup stages, redaction.
// ─────────────────────────────────────────────────────────────────────────────

fn dec(level: AuditLevel) -> PolicyDecision {
    PolicyDecision { level, omit_stages: vec![], omit_managed_fields: false }
}

#[test]
fn stage_emitter_drops_when_level_none() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::None), "acme".into());
    let r = e.build(AuditStage::RequestReceived, "alice", "get", "pods", "p", "ns",
                    "/api", 200, None, None);
    assert!(r.is_none());
}

#[test]
fn stage_emitter_emits_request_received() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::Metadata), "acme".into());
    let r = e.build(AuditStage::RequestReceived, "alice", "get", "pods", "p", "ns",
                    "/api", 200, None, None);
    assert!(r.is_some());
    assert_eq!(r.unwrap().audit_id, "aid");
}

#[test]
fn stage_emitter_dedup_same_stage() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::Metadata), "acme".into());
    let _ = e.build(AuditStage::RequestReceived, "alice", "get", "pods", "p", "ns",
                    "/api", 200, None, None);
    let r2 = e.build(AuditStage::RequestReceived, "alice", "get", "pods", "p", "ns",
                     "/api", 200, None, None);
    assert!(r2.is_none(), "duplicate stage emission must be suppressed");
}

#[test]
fn stage_emitter_progression_distinct_stages() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::Metadata), "acme".into());
    let s1 = e.build(AuditStage::RequestReceived, "a", "g", "p", "n", "ns", "/", 200, None, None);
    let s2 = e.build(AuditStage::ResponseStarted, "a", "g", "p", "n", "ns", "/", 200, None, None);
    let s3 = e.build(AuditStage::ResponseComplete, "a", "g", "p", "n", "ns", "/", 200, None, None);
    assert!(s1.is_some());
    assert!(s2.is_some());
    assert!(s3.is_some());
    assert_eq!(s1.unwrap().audit_id, s2.as_ref().unwrap().audit_id);
    assert_eq!(s2.unwrap().audit_id, s3.unwrap().audit_id);
}

#[test]
fn stage_emitter_skips_omit_stage() {
    let mut d = dec(AuditLevel::Metadata);
    d.omit_stages = vec![AuditStage::RequestReceived];
    let mut e = StageEmitter::new("aid".into(), d, "acme".into());
    let r = e.build(AuditStage::RequestReceived, "a", "g", "p", "n", "ns", "/", 200, None, None);
    assert!(r.is_none());
}

#[test]
fn stage_emitter_redacts_metadata_drops_request_response() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::Metadata), "acme".into());
    let r = e.build(AuditStage::ResponseComplete, "a", "g", "p", "n", "ns", "/", 200,
        Some(json!({"foo": 1})), Some(json!({"bar": 2})));
    let ev = r.unwrap();
    assert!(ev.request_object.is_none(), "Metadata level drops request body");
    assert!(ev.response_object.is_none());
}

#[test]
fn stage_emitter_request_level_keeps_request_drops_response() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::Request), "acme".into());
    let r = e.build(AuditStage::ResponseComplete, "a", "g", "p", "n", "ns", "/", 200,
        Some(json!({"foo": 1})), Some(json!({"bar": 2})));
    let ev = r.unwrap();
    assert!(ev.request_object.is_some());
    assert!(ev.response_object.is_none());
}

#[test]
fn stage_emitter_request_response_keeps_both() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::RequestResponse), "acme".into());
    let r = e.build(AuditStage::ResponseComplete, "a", "g", "p", "n", "ns", "/", 200,
        Some(json!({"foo": 1})), Some(json!({"bar": 2})));
    let ev = r.unwrap();
    assert!(ev.request_object.is_some());
    assert!(ev.response_object.is_some());
}

#[test]
fn stage_emitter_omit_managed_fields_strips() {
    let mut d = dec(AuditLevel::RequestResponse);
    d.omit_managed_fields = true;
    let mut e = StageEmitter::new("aid".into(), d, "acme".into());
    let r = e.build(AuditStage::ResponseComplete, "a", "g", "p", "n", "ns", "/", 200,
        Some(json!({"metadata": {"name": "x", "managedFields": [{"manager":"kubectl"}]}})),
        None);
    let ev = r.unwrap();
    let req = ev.request_object.unwrap();
    assert!(req["metadata"]["managedFields"].is_null(),
        "omitManagedFields must strip metadata.managedFields");
    assert_eq!(req["metadata"]["name"], "x", "other metadata preserved");
}

#[test]
fn stage_emitter_preserves_tenant_id_field() {
    let mut e = StageEmitter::new("aid".into(), dec(AuditLevel::Metadata), "acme".into());
    let r = e.build(AuditStage::RequestReceived, "alice", "get", "pods", "p", "ns",
                    "/api", 200, None, None).unwrap();
    assert_eq!(r.tenant_id, "acme",
        "tenant_id invariant: emitter MUST always tag the configured tenant_id");
}

// ─────────────────────────────────────────────────────────────────────────────
// Tenant invariant — rules MUST NOT select on tenant_id (it isn't a field on
// PolicyRule). This test guards by construction.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn policy_rule_has_no_tenant_field_at_compile_time() {
    let _r = PolicyRule::default();
    // If anyone adds a `tenant_id` field to PolicyRule, this test will refuse
    // to compile because the literal below would need a new initializer.
    let _full = PolicyRule {
        level: AuditLevel::Metadata,
        users: vec![], user_groups: vec![], verbs: vec![],
        resources: vec![], namespaces: vec![], non_resource_urls: vec![],
        omit_stages: vec![], omit_managed_fields: false,
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on richer body parsing or audit sink wiring.
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[cfg(feature = "live-integration")]
fn policy_yaml_round_trip_v1() {
    // pending: port `audit/v1/types.go` YAML decode + re-encode against fixtures
}

#[test] #[cfg(feature = "live-integration")]
fn policy_v1beta1_to_v1_conversion() {
    // pending: requires audit/v1beta1 conversion shim
}

#[test] #[cfg(feature = "live-integration")]
fn audit_id_propagation_through_x_audit_id_header() {
    // pending: requires HTTP middleware — accept incoming X-Audit-ID header
}

#[test] #[cfg(feature = "live-integration")]
fn webhook_audit_backend_fan_out() {
    // pending: requires webhook backend — emit to remote sink with retries
}

#[test] #[cfg(feature = "live-integration")]
fn dynamic_audit_sink_via_auditsink_resource() {
    // pending: requires auditregistration.k8s.io/v1alpha1 (deprecated path)
}

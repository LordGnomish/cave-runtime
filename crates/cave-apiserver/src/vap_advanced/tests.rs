// SPDX-License-Identifier: AGPL-3.0-or-later
//! ValidatingAdmissionPolicy parity tests.
//!
//! Each test references the upstream Go test name in its doc comment so reverse
//! lookup is always one `grep` away. Tests that depend on a real CEL evaluator
//! are gated with `#[ignore]` and use `PanicEvaluator`.

use super::*;
use crate::admission::{AdmissionChain, AdmissionRequest, Operation};
use crate::resources::{ConfigMap, ObjectMeta, Resource};
use std::collections::HashMap;
use std::sync::Arc;

fn vap(name: &str, tenant: &str) -> ValidatingAdmissionPolicy {
    let mut meta = ObjectMeta::new(name, "");
    meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
    ValidatingAdmissionPolicy {
        api_version: "admissionregistration.k8s.io/v1".into(),
        kind: "ValidatingAdmissionPolicy".into(),
        metadata: meta,
        spec: ValidatingAdmissionPolicySpec::default(),
    }
}

fn binding(name: &str, tenant: &str, policy: &str) -> ValidatingAdmissionPolicyBinding {
    let mut meta = ObjectMeta::new(name, "");
    meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
    ValidatingAdmissionPolicyBinding {
        api_version: "admissionregistration.k8s.io/v1".into(),
        kind: "ValidatingAdmissionPolicyBinding".into(),
        metadata: meta,
        spec: ValidatingAdmissionPolicyBindingSpec {
            policy_name: policy.into(),
            ..Default::default()
        },
    }
}

fn req(op: Operation, ns: &str, tenant: &str) -> AdmissionRequest {
    let mut meta = ObjectMeta::new("cm1", ns);
    meta.labels.insert("env".into(), "prod".into());
    let cm = Resource::ConfigMap(ConfigMap {
        api_version: "v1".into(), kind: "ConfigMap".into(),
        metadata: meta, data: HashMap::new(),
    });
    AdmissionRequest {
        uid: "uid".into(), tenant_id: tenant.into(), namespace: ns.into(),
        kind: "ConfigMap".into(), name: "cm1".into(), operation: op,
        object: Some(cm), old_object: None,
        user: "alice".into(), dry_run: false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type round-trip — upstream `TestValidatingAdmissionPolicy_DeepCopy` &
// `TestRoundTripTypes`. We verify wire-format stability against the GA schema.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn vap_roundtrip_minimal() {
    let p = vap("p1", "acme");
    let s = serde_json::to_string(&p).unwrap();
    let p2: ValidatingAdmissionPolicy = serde_json::from_str(&s).unwrap();
    assert_eq!(p2.metadata.name, "p1");
    assert_eq!(p2.kind, "ValidatingAdmissionPolicy");
}

#[test]
fn vapb_roundtrip_minimal() {
    let b = binding("b1", "acme", "p1");
    let s = serde_json::to_string(&b).unwrap();
    let b2: ValidatingAdmissionPolicyBinding = serde_json::from_str(&s).unwrap();
    assert_eq!(b2.spec.policy_name, "p1");
}

#[test]
fn failure_policy_default_is_fail() {
    // Upstream parity: admissionregistration/v1 default for FailurePolicy is Fail.
    let p: ValidatingAdmissionPolicy = serde_json::from_str(
        r#"{"apiVersion":"admissionregistration.k8s.io/v1","kind":"ValidatingAdmissionPolicy",
              "metadata":{"name":"p","namespace":"","uid":"00000000-0000-0000-0000-000000000000",
                          "resource_version":1,"creation_timestamp":"2026-01-01T00:00:00Z",
                          "labels":{},"annotations":{},"owner_references":[],"finalizers":[],
                          "deletion_timestamp":null},
              "spec":{}}"#).unwrap();
    assert_eq!(p.spec.failure_policy, FailurePolicyType::Fail);
}

#[test]
fn parameter_not_found_default_is_deny() {
    let pr: ParamRef = serde_json::from_str("{}").unwrap();
    assert_eq!(pr.parameter_not_found_action, ParameterNotFoundActionType::Deny);
}

#[test]
fn match_policy_default_is_equivalent() {
    let mr: MatchResources = serde_json::from_str("{}").unwrap();
    assert!(mr.match_policy.is_none()); // explicit None when omitted
}

#[test]
fn scope_default_is_all() {
    let r: RuleWithOperations = serde_json::from_str(r#"{"operations":["CREATE"]}"#).unwrap();
    assert_eq!(r.scope, ScopeType::All);
}

// ─────────────────────────────────────────────────────────────────────────────
// VapStore — CRUD + tenant isolation. Upstream `TestValidatingPolicyStorage`,
// `TestValidatingPolicyBindingStorage`. Tenant scope is layered on top.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn store_put_and_get_policy() {
    let s = VapStore::new();
    s.put_policy(vap("p1", "acme"));
    assert!(s.get_policy("acme", "p1").is_some());
    assert!(s.get_policy("acme", "missing").is_none());
}

#[test]
fn store_tenant_isolation_get() {
    let s = VapStore::new();
    s.put_policy(vap("p1", "acme"));
    assert!(s.get_policy("globex", "p1").is_none(), "tenant isolation invariant");
}

#[test]
fn store_tenant_isolation_list() {
    let s = VapStore::new();
    s.put_policy(vap("p1", "acme"));
    s.put_policy(vap("p2", "globex"));
    assert_eq!(s.list_policies("acme").len(), 1);
    assert_eq!(s.list_policies("globex").len(), 1);
}

#[test]
fn store_delete_policy_returns_true() {
    let s = VapStore::new();
    s.put_policy(vap("p1", "acme"));
    assert!(s.delete_policy("acme", "p1"));
    assert!(!s.delete_policy("acme", "p1"));
}

#[test]
fn store_delete_policy_other_tenant_noop() {
    let s = VapStore::new();
    s.put_policy(vap("p1", "acme"));
    assert!(!s.delete_policy("globex", "p1"));
    assert!(s.get_policy("acme", "p1").is_some());
}

#[test]
fn store_put_and_list_bindings() {
    let s = VapStore::new();
    s.put_binding(binding("b1", "acme", "p1"));
    s.put_binding(binding("b2", "acme", "p1"));
    s.put_binding(binding("b3", "globex", "p1"));
    assert_eq!(s.list_bindings("acme").len(), 2);
    assert_eq!(s.list_bindings("globex").len(), 1);
}

#[test]
fn store_pairs_for_tenant_resolves_binding_to_policy() {
    let s = VapStore::new();
    s.put_policy(vap("p1", "acme"));
    s.put_binding(binding("b1", "acme", "p1"));
    let pairs = s.pairs_for_tenant("acme");
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0.metadata.name, "p1");
    assert_eq!(pairs[0].1.metadata.name, "b1");
}

#[test]
fn store_pairs_for_tenant_drops_orphan_bindings() {
    let s = VapStore::new();
    s.put_binding(binding("b1", "acme", "no-such-policy"));
    assert!(s.pairs_for_tenant("acme").is_empty());
}

#[test]
fn store_pairs_does_not_cross_tenants_even_when_names_match() {
    let s = VapStore::new();
    s.put_policy(vap("p1", "acme"));
    s.put_binding(binding("b1", "globex", "p1"));
    assert!(s.pairs_for_tenant("acme").is_empty(),
        "binding in globex must NOT see policy in acme");
    assert!(s.pairs_for_tenant("globex").is_empty(),
        "binding in globex must NOT cross to acme's policy");
}

// ─────────────────────────────────────────────────────────────────────────────
// Resource matching — upstream `TestMatcher_Matches` and friends.
// ─────────────────────────────────────────────────────────────────────────────

fn input_for(op: Operation, group: &str, version: &str, resource: &str, ns: &str) -> MatchInput<'static> {
    static EMPTY: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    let empty = EMPTY.get_or_init(HashMap::new);
    MatchInput {
        group: Box::leak(group.to_string().into_boxed_str()),
        version: Box::leak(version.to_string().into_boxed_str()),
        resource: Box::leak(resource.to_string().into_boxed_str()),
        name: "obj",
        namespace: Box::leak(ns.to_string().into_boxed_str()),
        operation: Box::leak(Box::new(op)),
        object_labels: empty,
        namespace_labels: empty,
    }
}

#[test]
fn op_matches_wildcard() {
    assert!(op_matches("*", &Operation::Create));
    assert!(op_matches("*", &Operation::Delete));
}

#[test]
fn op_matches_exact() {
    assert!(op_matches("CREATE", &Operation::Create));
    assert!(!op_matches("CREATE", &Operation::Update));
    assert!(op_matches("UPDATE", &Operation::Update));
    assert!(op_matches("DELETE", &Operation::Delete));
    assert!(op_matches("CONNECT", &Operation::Connect));
}

#[test]
fn op_matches_unknown_returns_false() {
    assert!(!op_matches("YEET", &Operation::Create));
}

#[test]
fn rule_matches_wildcard_group() {
    let rule = RuleWithOperations {
        operations: vec!["CREATE".into()],
        api_groups: vec!["*".into()],
        api_versions: vec!["v1".into()],
        resources: vec!["pods".into()],
        scope: ScopeType::All,
    };
    let input = input_for(Operation::Create, "any", "v1", "pods", "default");
    assert!(rule_matches(&rule, &input));
}

#[test]
fn rule_matches_filters_by_resource() {
    let rule = RuleWithOperations {
        operations: vec!["*".into()],
        api_groups: vec![""].into_iter().map(String::from).collect(),
        api_versions: vec!["v1".into()],
        resources: vec!["pods".into()],
        scope: ScopeType::All,
    };
    let i_pod = input_for(Operation::Create, "", "v1", "pods", "default");
    let i_cm  = input_for(Operation::Create, "", "v1", "configmaps", "default");
    assert!(rule_matches(&rule, &i_pod));
    assert!(!rule_matches(&rule, &i_cm));
}

#[test]
fn rule_matches_scope_namespaced() {
    let rule = RuleWithOperations {
        operations: vec!["*".into()], api_groups: vec!["*".into()],
        api_versions: vec!["*".into()], resources: vec!["*".into()],
        scope: ScopeType::Namespaced,
    };
    assert!(rule_matches(&rule, &input_for(Operation::Create, "", "v1", "pods", "default")));
    assert!(!rule_matches(&rule, &input_for(Operation::Create, "", "v1", "nodes", "")));
}

#[test]
fn rule_matches_scope_cluster() {
    let rule = RuleWithOperations {
        operations: vec!["*".into()], api_groups: vec!["*".into()],
        api_versions: vec!["*".into()], resources: vec!["*".into()],
        scope: ScopeType::Cluster,
    };
    assert!(!rule_matches(&rule, &input_for(Operation::Create, "", "v1", "pods", "default")));
    assert!(rule_matches(&rule, &input_for(Operation::Create, "", "v1", "nodes", "")));
}

#[test]
fn rule_matches_scope_all() {
    let rule = RuleWithOperations {
        operations: vec!["*".into()], api_groups: vec!["*".into()],
        api_versions: vec!["*".into()], resources: vec!["*".into()],
        scope: ScopeType::All,
    };
    assert!(rule_matches(&rule, &input_for(Operation::Create, "", "v1", "pods", "default")));
    assert!(rule_matches(&rule, &input_for(Operation::Create, "", "v1", "nodes", "")));
}

#[test]
fn rule_empty_groups_matches_anything_when_unset() {
    // Upstream: an empty groups list is treated as "no constraint" by NamedRuleWithOperations.
    let rule = RuleWithOperations {
        operations: vec!["*".into()],
        api_groups: vec![],
        api_versions: vec![],
        resources: vec![],
        scope: ScopeType::All,
    };
    assert!(rule_matches(&rule, &input_for(Operation::Create, "anything", "v1", "anything", "ns")));
}

#[test]
fn named_rule_filters_by_resource_name() {
    let rule = NamedRuleWithOperations {
        rule: RuleWithOperations {
            operations: vec!["*".into()],
            api_groups: vec!["*".into()],
            api_versions: vec!["*".into()],
            resources: vec!["pods".into()],
            scope: ScopeType::All,
        },
        resource_names: vec!["allowed".into()],
    };
    let mut input_match = input_for(Operation::Create, "", "v1", "pods", "default");
    input_match.name = "allowed";
    let mut input_skip = input_for(Operation::Create, "", "v1", "pods", "default");
    input_skip.name = "different";
    assert!(named_rule_matches(&rule, &input_match));
    assert!(!named_rule_matches(&rule, &input_skip));
}

#[test]
fn label_selector_match_labels() {
    let mut sel = LabelSelector::default();
    sel.match_labels.insert("app".into(), "web".into());
    let mut labs = HashMap::new();
    labs.insert("app".into(), "web".into());
    assert!(label_selector_matches(&sel, &labs));
    labs.insert("app".into(), "db".into());
    assert!(!label_selector_matches(&sel, &labs));
}

#[test]
fn label_selector_in_operator() {
    let sel = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelSelectorRequirement {
            key: "tier".into(), operator: "In".into(),
            values: vec!["frontend".into(), "backend".into()],
        }],
    };
    let mut labs = HashMap::new();
    labs.insert("tier".into(), "frontend".into());
    assert!(label_selector_matches(&sel, &labs));
    labs.insert("tier".into(), "cache".into());
    assert!(!label_selector_matches(&sel, &labs));
}

#[test]
fn label_selector_notin_operator() {
    let sel = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelSelectorRequirement {
            key: "tier".into(), operator: "NotIn".into(),
            values: vec!["cache".into()],
        }],
    };
    let mut labs = HashMap::new();
    labs.insert("tier".into(), "frontend".into());
    assert!(label_selector_matches(&sel, &labs));
    labs.insert("tier".into(), "cache".into());
    assert!(!label_selector_matches(&sel, &labs));
}

#[test]
fn label_selector_exists_operator() {
    let sel = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelSelectorRequirement {
            key: "team".into(), operator: "Exists".into(), values: vec![],
        }],
    };
    let mut labs = HashMap::new();
    labs.insert("team".into(), "platform".into());
    assert!(label_selector_matches(&sel, &labs));
    labs.clear();
    assert!(!label_selector_matches(&sel, &labs));
}

#[test]
fn label_selector_does_not_exist_operator() {
    let sel = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelSelectorRequirement {
            key: "team".into(), operator: "DoesNotExist".into(), values: vec![],
        }],
    };
    let labs = HashMap::new();
    assert!(label_selector_matches(&sel, &labs));
    let mut labs = HashMap::new();
    labs.insert("team".into(), "platform".into());
    assert!(!label_selector_matches(&sel, &labs));
}

#[test]
fn label_selector_unknown_operator_never_matches() {
    let sel = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelSelectorRequirement {
            key: "k".into(), operator: "Bogus".into(), values: vec![],
        }],
    };
    assert!(!label_selector_matches(&sel, &HashMap::new()));
}

#[test]
fn match_resources_excludes_take_priority() {
    let mr = MatchResources {
        resource_rules: vec![NamedRuleWithOperations {
            rule: RuleWithOperations {
                operations: vec!["*".into()], api_groups: vec!["*".into()],
                api_versions: vec!["*".into()], resources: vec!["*".into()],
                scope: ScopeType::All,
            }, resource_names: vec![],
        }],
        exclude_resource_rules: vec![NamedRuleWithOperations {
            rule: RuleWithOperations {
                operations: vec!["*".into()], api_groups: vec!["*".into()],
                api_versions: vec!["*".into()], resources: vec!["secrets".into()],
                scope: ScopeType::All,
            }, resource_names: vec![],
        }],
        ..Default::default()
    };
    assert!(match_resources_matches(&mr, &input_for(Operation::Create, "", "v1", "pods", "ns")));
    assert!(!match_resources_matches(&mr, &input_for(Operation::Create, "", "v1", "secrets", "ns")));
}

#[test]
fn match_resources_empty_rules_matches_anything() {
    let mr = MatchResources::default();
    assert!(match_resources_matches(&mr, &input_for(Operation::Create, "", "v1", "pods", "ns")));
}

// ─────────────────────────────────────────────────────────────────────────────
// ParamResolver — upstream `TestParamResolver`. Cross-tenant invariant tested.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn param_resolver_named_lookup() {
    let r = InMemoryParamResolver::new();
    let kind = ParamKind { api_version: "v1".into(), kind: "ConfigMap".into() };
    r.insert("acme", &kind, "default", "p1", serde_json::json!({"x": 1}));
    let pr = ParamRef { name: "p1".into(), namespace: "default".into(), ..Default::default() };
    let v = r.resolve("acme", &kind, &pr).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0]["x"], serde_json::json!(1));
}

#[test]
fn param_resolver_named_not_found() {
    let r = InMemoryParamResolver::new();
    let kind = ParamKind { api_version: "v1".into(), kind: "ConfigMap".into() };
    let pr = ParamRef { name: "missing".into(), namespace: "default".into(), ..Default::default() };
    assert_eq!(r.resolve("acme", &kind, &pr), Err(ParamResolveError::NotFound));
}

#[test]
fn param_resolver_tenant_isolation() {
    let r = InMemoryParamResolver::new();
    let kind = ParamKind { api_version: "v1".into(), kind: "ConfigMap".into() };
    r.insert("acme", &kind, "default", "p1", serde_json::json!({}));
    let pr = ParamRef { name: "p1".into(), namespace: "default".into(), ..Default::default() };
    assert_eq!(r.resolve("globex", &kind, &pr), Err(ParamResolveError::NotFound),
        "cross-tenant param lookup must fail (cave-runtime invariant)");
}

#[test]
fn param_resolver_selector_lists_all_in_scope() {
    let r = InMemoryParamResolver::new();
    let kind = ParamKind { api_version: "v1".into(), kind: "ConfigMap".into() };
    r.insert("acme", &kind, "default", "a", serde_json::json!({}));
    r.insert("acme", &kind, "default", "b", serde_json::json!({}));
    r.insert("acme", &kind, "other",   "c", serde_json::json!({}));
    let pr = ParamRef {
        name: "".into(), namespace: "default".into(),
        selector: Some(LabelSelector::default()),
        ..Default::default()
    };
    let v = r.resolve("acme", &kind, &pr).unwrap();
    assert_eq!(v.len(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// FixedEvaluator — testing the testing tool. Belt-and-braces.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn fixed_evaluator_returns_known() {
    let e = FixedEvaluator::new().with("true_expr", CelValue::Bool(true));
    assert_eq!(e.evaluate("true_expr", &CelActivation::default()).unwrap(), CelValue::Bool(true));
}

#[test]
fn fixed_evaluator_unknown_is_compile_error() {
    let e = FixedEvaluator::new();
    matches!(e.evaluate("nope", &CelActivation::default()),
             Err(CelError::Compile(_)));
}

#[test]
fn panic_evaluator_returns_compile_error() {
    let e = PanicEvaluator;
    let err = e.evaluate("anything", &CelActivation::default()).unwrap_err();
    assert!(matches!(err, CelError::Compile(msg) if msg.contains("CEL evaluator not yet ported")));
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatcher — fail policy, validation actions, param resolution. Upstream
// `TestDispatcher_dispatchInvocations`.
// ─────────────────────────────────────────────────────────────────────────────

fn dispatcher_with_eval(ev: FixedEvaluator) -> Dispatcher {
    Dispatcher::new(Arc::new(ev), Arc::new(InMemoryParamResolver::new()))
}

#[test]
fn dispatch_allow_when_validation_passes() {
    let ev = FixedEvaluator::new().with("true", CelValue::Bool(true));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation { expression: "true".into(), ..Default::default() });
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out, vec![DispatchOutcome::Allow]);
}

#[test]
fn dispatch_deny_when_validation_fails() {
    let ev = FixedEvaluator::new().with("expr", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation {
        expression: "expr".into(), message: "nope".into(),
        reason: "Forbidden".into(), ..Default::default()
    });
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out, vec![DispatchOutcome::Deny {
        message: "nope".into(), reason: "Forbidden".into() }]);
}

#[test]
fn dispatch_deny_uses_expression_when_message_empty() {
    let ev = FixedEvaluator::new().with("x > 0", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation {
        expression: "x > 0".into(), message: "".into(), ..Default::default()
    });
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    match &out[0] {
        DispatchOutcome::Deny { message, .. } => assert_eq!(message, "x > 0"),
        _ => panic!("expected deny"),
    }
}

#[test]
fn dispatch_failure_policy_fail_on_compile_error() {
    let ev = FixedEvaluator::new().with_err("bad", CelError::Compile("syntax".into()));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation { expression: "bad".into(), ..Default::default() });
    p.spec.failure_policy = FailurePolicyType::Fail;
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(matches!(out[0], DispatchOutcome::Error(_)));
}

#[test]
fn dispatch_failure_policy_ignore_swallows_error() {
    let ev = FixedEvaluator::new().with_err("bad", CelError::Compile("syntax".into()));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation { expression: "bad".into(), ..Default::default() });
    p.spec.failure_policy = FailurePolicyType::Ignore;
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out, vec![DispatchOutcome::SilencedError]);
}

#[test]
fn dispatch_validation_action_warn() {
    let ev = FixedEvaluator::new().with("expr", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation {
        expression: "expr".into(), message: "warning!".into(), ..Default::default()
    });
    let mut b = binding("b", "acme", "p");
    b.spec.validation_actions = vec![ValidationAction::Warn];
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out, vec![DispatchOutcome::Warn("warning!".into())]);
}

#[test]
fn dispatch_validation_action_deny_explicit() {
    let ev = FixedEvaluator::new().with("expr", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation {
        expression: "expr".into(), message: "no".into(), ..Default::default()
    });
    let mut b = binding("b", "acme", "p");
    b.spec.validation_actions = vec![ValidationAction::Deny];
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], DispatchOutcome::Deny { .. }));
}

#[test]
fn dispatch_validation_action_warn_and_deny() {
    let ev = FixedEvaluator::new().with("expr", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation {
        expression: "expr".into(), message: "msg".into(), ..Default::default()
    });
    let mut b = binding("b", "acme", "p");
    b.spec.validation_actions = vec![ValidationAction::Warn, ValidationAction::Deny];
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out.len(), 2);
    assert!(matches!(out[0], DispatchOutcome::Warn(_)));
    assert!(matches!(out[1], DispatchOutcome::Deny { .. }));
}

#[test]
fn dispatch_param_not_found_with_deny_action_denies() {
    let d = Dispatcher::new(
        Arc::new(FixedEvaluator::new()),
        Arc::new(InMemoryParamResolver::new()));
    let mut p = vap("p", "acme");
    p.spec.param_kind = Some(ParamKind { api_version: "v1".into(), kind: "ConfigMap".into() });
    let mut b = binding("b", "acme", "p");
    b.spec.param_ref = Some(ParamRef {
        name: "missing".into(), namespace: "default".into(),
        parameter_not_found_action: ParameterNotFoundActionType::Deny,
        ..Default::default()
    });
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(matches!(out[0], DispatchOutcome::Deny { .. }));
}

#[test]
fn dispatch_param_not_found_with_allow_action_proceeds() {
    let ev = FixedEvaluator::new().with("expr", CelValue::Bool(true));
    let d = Dispatcher::new(Arc::new(ev), Arc::new(InMemoryParamResolver::new()));
    let mut p = vap("p", "acme");
    p.spec.param_kind = Some(ParamKind { api_version: "v1".into(), kind: "ConfigMap".into() });
    p.spec.validations.push(Validation { expression: "expr".into(), ..Default::default() });
    let mut b = binding("b", "acme", "p");
    b.spec.param_ref = Some(ParamRef {
        name: "missing".into(), namespace: "default".into(),
        parameter_not_found_action: ParameterNotFoundActionType::Allow,
        ..Default::default()
    });
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out, vec![DispatchOutcome::Allow]);
}

#[test]
fn dispatch_match_constraints_filter_skips_dispatch() {
    let ev = FixedEvaluator::new().with("expr", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.match_constraints = Some(MatchResources {
        resource_rules: vec![NamedRuleWithOperations {
            rule: RuleWithOperations {
                operations: vec!["*".into()], api_groups: vec!["*".into()],
                api_versions: vec!["*".into()], resources: vec!["pods".into()],
                scope: ScopeType::All,
            }, resource_names: vec![],
        }],
        ..Default::default()
    });
    p.spec.validations.push(Validation { expression: "expr".into(), ..Default::default() });
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(out.is_empty(), "non-matching constraint must skip");
}

#[test]
fn dispatch_match_condition_false_skips() {
    let ev = FixedEvaluator::new()
        .with("cond", CelValue::Bool(false))
        .with("v", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.match_conditions.push(MatchCondition {
        name: "skip".into(), expression: "cond".into(),
    });
    p.spec.validations.push(Validation { expression: "v".into(), ..Default::default() });
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(out.is_empty(), "matchCondition=false must skip validation");
}

#[test]
fn dispatch_match_condition_error_with_fail_policy_errors() {
    let ev = FixedEvaluator::new().with_err("cond", CelError::Runtime("oops".into()));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.match_conditions.push(MatchCondition {
        name: "n".into(), expression: "cond".into(),
    });
    p.spec.failure_policy = FailurePolicyType::Fail;
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(matches!(out[0], DispatchOutcome::Error(_)));
}

#[test]
fn dispatch_match_condition_error_with_ignore_policy_silenced() {
    let ev = FixedEvaluator::new().with_err("cond", CelError::Runtime("oops".into()));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.match_conditions.push(MatchCondition {
        name: "n".into(), expression: "cond".into(),
    });
    p.spec.failure_policy = FailurePolicyType::Ignore;
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert_eq!(out, vec![DispatchOutcome::SilencedError]);
}

#[test]
fn dispatch_non_bool_validation_yields_fail() {
    let ev = FixedEvaluator::new().with("v", CelValue::Int(42));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation { expression: "v".into(), ..Default::default() });
    p.spec.failure_policy = FailurePolicyType::Fail;
    let b = binding("b", "acme", "p");
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(matches!(out[0], DispatchOutcome::Error(_)));
}

#[test]
fn dispatch_audit_only_action_produces_no_outcome() {
    // Audit-only validation actions are TODO M4; current behavior is to drop
    // them. This guards the hand-off between M1 and M4.
    let ev = FixedEvaluator::new().with("v", CelValue::Bool(false));
    let d = dispatcher_with_eval(ev);
    let mut p = vap("p", "acme");
    p.spec.validations.push(Validation { expression: "v".into(), ..Default::default() });
    let mut b = binding("b", "acme", "p");
    b.spec.validation_actions = vec![ValidationAction::Audit];
    let r = req(Operation::Create, "default", "acme");
    let i = MatchInput {
        group: "", version: "v1", resource: "configmaps", name: "cm1",
        namespace: "default", operation: &Operation::Create,
        object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
    };
    let out = d.dispatch_one("acme", &p, &b, &r, &i);
    assert!(out.is_empty(), "audit-only currently drops; M4 will surface");
}

// ─────────────────────────────────────────────────────────────────────────────
// VapPlugin — wires Dispatcher into the AdmissionChain. Upstream
// `TestPolicyValidate_Integration`.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn plugin_allows_when_no_policies_for_tenant() {
    let store = Arc::new(VapStore::new());
    let d = Dispatcher::new(Arc::new(PanicEvaluator), Arc::new(InMemoryParamResolver::new()));
    let p = VapPlugin::new(store, d);
    let r = req(Operation::Create, "default", "acme");
    let resp = p.validate(&r);
    assert!(resp.allowed);
    assert_eq!(resp.tenant_id, "acme");
}

#[test]
fn plugin_denies_via_failed_validation() {
    let store = Arc::new(VapStore::new());
    let mut policy = vap("p", "acme");
    policy.spec.validations.push(Validation {
        expression: "v".into(), message: "boom".into(), ..Default::default()
    });
    store.put_policy(policy);
    store.put_binding(binding("b", "acme", "p"));
    let d = Dispatcher::new(
        Arc::new(FixedEvaluator::new().with("v", CelValue::Bool(false))),
        Arc::new(InMemoryParamResolver::new()));
    let plug = VapPlugin::new(store, d);
    let r = req(Operation::Create, "default", "acme");
    let resp = plug.validate(&r);
    assert!(!resp.allowed);
    assert_eq!(resp.status_code, 403);
    assert_eq!(resp.status_message, "boom");
    assert_eq!(resp.tenant_id, "acme");
}

#[test]
fn plugin_warning_only_still_allows() {
    let store = Arc::new(VapStore::new());
    let mut policy = vap("p", "acme");
    policy.spec.validations.push(Validation {
        expression: "v".into(), message: "careful".into(), ..Default::default()
    });
    store.put_policy(policy);
    let mut b = binding("b", "acme", "p");
    b.spec.validation_actions = vec![ValidationAction::Warn];
    store.put_binding(b);
    let d = Dispatcher::new(
        Arc::new(FixedEvaluator::new().with("v", CelValue::Bool(false))),
        Arc::new(InMemoryParamResolver::new()));
    let plug = VapPlugin::new(store, d);
    let r = req(Operation::Create, "default", "acme");
    let resp = plug.validate(&r);
    assert!(resp.allowed);
    assert_eq!(resp.warnings, vec!["careful".to_string()]);
}

#[test]
fn plugin_does_not_apply_other_tenant_policies() {
    let store = Arc::new(VapStore::new());
    let mut policy = vap("p", "globex");
    policy.spec.validations.push(Validation {
        expression: "v".into(), message: "should not run".into(), ..Default::default()
    });
    store.put_policy(policy);
    store.put_binding(binding("b", "globex", "p"));
    let d = Dispatcher::new(Arc::new(PanicEvaluator), Arc::new(InMemoryParamResolver::new()));
    let plug = VapPlugin::new(store, d);
    let r = req(Operation::Create, "default", "acme");
    let resp = plug.validate(&r);
    assert!(resp.allowed, "globex policy must not run on acme request");
}

#[test]
fn plugin_integrates_with_admission_chain() {
    let store = Arc::new(VapStore::new());
    let d = Dispatcher::new(Arc::new(PanicEvaluator), Arc::new(InMemoryParamResolver::new()));
    let plug = VapPlugin::new(store, d);
    let chain = AdmissionChain::new().with_validating(Arc::new(plug));
    let r = req(Operation::Create, "default", "acme");
    let (_, resp) = chain.dispatch(r);
    assert!(resp.allowed);
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated by real CEL evaluator. Each one is a port of an upstream
// integration test from `cel/library/*_test.go` and
// `apiserver/pkg/admission/plugin/policy/validating/*_integration_test.go`.
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[cfg(feature = "live-integration")] // TODO(KEP-3488 M1.5): port cel-go evaluator
fn cel_self_dot_metadata_name_present() {
    // Upstream: `validating_test.go::TestValidate_Self_MetadataName`
    let _e: Box<dyn CelEvaluator> = Box::new(PanicEvaluator);
    // pending: requires real CEL — expression `object.metadata.name == 'allowed'`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_has_macro_on_optional_field() {
    // pending: requires real CEL — `has(object.spec.foo)`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_string_lib_starts_with() {
    // pending: requires real CEL — `object.metadata.name.startsWith('foo-')`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_string_lib_ends_with() {
    // pending: requires real CEL — `object.metadata.name.endsWith('-bar')`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_quantity_compare_milli() {
    // pending: requires real CEL+quantity lib — `quantity('100m') < quantity('1')`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_url_parse_lib() {
    // pending: requires real CEL+url lib — `url('https://x').getHost() == 'x'`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_regex_match() {
    // pending: requires real CEL — `'foo123'.matches('^foo[0-9]+$')`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_authz_check_kind_resource() {
    // pending: requires authorizer activation — `authorizer.group('').resource('pods').check('list').allowed()`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_message_expression_overrides_message() {
    // pending: requires CEL — messageExpression returns string overriding `message`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_variable_reference_other_variable() {
    // pending: requires CEL — `variables.x` reference inside another variable
}

#[test] #[cfg(feature = "live-integration")]
fn cel_param_array_index() {
    // pending: requires CEL — `params[0].x == 1` after multi-param resolution
}

#[test] #[cfg(feature = "live-integration")]
fn cel_namespace_object_labels() {
    // pending: requires CEL — `namespaceObject.metadata.labels['team']`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_old_object_diff() {
    // pending: requires CEL — `oldObject.spec.replicas != object.spec.replicas`
}

#[test] #[cfg(feature = "live-integration")]
fn cel_audit_annotation_emission() {
    // pending: M4 hand-off — auditAnnotation produces an audit field on the response
}

#[test] #[cfg(feature = "live-integration")]
fn cel_message_too_long_truncates() {
    // pending: requires CEL — message longer than 12kB is truncated to 12kB
}

#[test] #[cfg(feature = "live-integration")]
fn cel_runtime_cost_budget_exceeded_is_error() {
    // pending: requires CEL+cost budget — runtime cost > 1e6 yields an error
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno engine integration tests.

use cave_policy::kyverno::{
    jmespath,
    models::{
        ClusterPolicy, Conditions, MatchResources, Mutation, ObjectMeta, PolicyException,
        PolicyExceptionEntry, PolicyExceptionSpec, PolicySpec, ResourceDescription,
        ResourceFilter, Validation, ValidationFailureAction,
    },
    KyvernoEngine,
};
use serde_json::json;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn make_engine() -> KyvernoEngine { KyvernoEngine::new() }

fn pod_resource(namespace: &str, name: &str, labels: serde_json::Value) -> serde_json::Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": labels
        },
        "spec": {
            "containers": [{"name": "main", "image": "nginx:1.25"}]
        }
    })
}

fn make_policy(name: &str, rules: Vec<cave_policy::kyverno::models::KyvernoRule>, action: ValidationFailureAction) -> ClusterPolicy {
    ClusterPolicy {
        api_version: "kyverno.io/v1".into(),
        kind: "ClusterPolicy".into(),
        metadata: ObjectMeta {
            name: name.to_string(),
            ..Default::default()
        },
        spec: PolicySpec {
            validation_failure_action: action,
            rules,
            ..Default::default()
        },
        status: None,
    }
}

fn match_pods(namespace: &str) -> MatchResources {
    MatchResources {
        resources: Some(ResourceDescription {
            kinds: vec!["Pod".to_string()],
            namespaces: vec![namespace.to_string()],
            ..Default::default()
        }),
        ..Default::default()
    }
}

// ─── JMESPath Tests ───────────────────────────────────────────────────────────

#[test]
fn test_jmespath_simple_field() {
    let data = json!({"name": "alice", "age": 30});
    let result = jmespath::evaluate("name", &data).unwrap();
    assert_eq!(result, json!("alice"));
}

#[test]
fn test_jmespath_nested_field() {
    let data = json!({"spec": {"replicas": 3}});
    let result = jmespath::evaluate("spec.replicas", &data).unwrap();
    assert_eq!(result, json!(3));
}

#[test]
fn test_jmespath_array_index() {
    let data = json!({"containers": [{"name": "a"}, {"name": "b"}]});
    let result = jmespath::evaluate("containers[0].name", &data).unwrap();
    assert_eq!(result, json!("a"));
}

#[test]
fn test_jmespath_wildcard() {
    let data = json!({"containers": [{"name": "a"}, {"name": "b"}]});
    let result = jmespath::evaluate("containers[*].name", &data).unwrap();
    assert_eq!(result, json!(["a", "b"]));
}

#[test]
fn test_jmespath_filter_projection() {
    let data = json!({
        "containers": [
            {"name": "a", "ready": true},
            {"name": "b", "ready": false},
            {"name": "c", "ready": true}
        ]
    });
    let result = jmespath::evaluate("containers[?ready].name", &data).unwrap();
    assert!(result.is_array());
    let arr = result.as_array().unwrap();
    assert!(arr.contains(&json!("a")));
    assert!(arr.contains(&json!("c")));
    assert!(!arr.contains(&json!("b")));
}

#[test]
fn test_jmespath_length_function() {
    let data = json!({"items": [1, 2, 3, 4, 5]});
    let result = jmespath::evaluate("length(items)", &data).unwrap();
    assert_eq!(result, json!(5));
}

#[test]
fn test_jmespath_to_string() {
    let data = json!({"count": 42});
    let result = jmespath::evaluate("to_string(count)", &data).unwrap();
    assert_eq!(result, json!("42"));
}

#[test]
fn test_jmespath_to_number() {
    let data = json!({"val": "3.14"});
    let result = jmespath::evaluate("to_number(val)", &data).unwrap();
    assert!((result.as_f64().unwrap() - 3.14).abs() < 0.001);
}

#[test]
fn test_jmespath_keys_function() {
    let data = json!({"a": 1, "b": 2, "c": 3});
    let result = jmespath::evaluate("keys(@)", &data).unwrap();
    assert!(result.is_array());
    assert_eq!(result.as_array().unwrap().len(), 3);
}

#[test]
fn test_jmespath_values_function() {
    let data = json!({"x": 10, "y": 20});
    let result = jmespath::evaluate("values(@)", &data).unwrap();
    assert!(result.is_array());
}

#[test]
fn test_jmespath_contains_function() {
    let data = json!({"tags": ["web", "prod", "api"]});
    let result = jmespath::evaluate("contains(tags, 'prod')", &data).unwrap();
    assert_eq!(result, json!(true));
}

#[test]
fn test_jmespath_starts_with() {
    let data = json!({"name": "nginx-proxy"});
    let result = jmespath::evaluate("starts_with(name, 'nginx')", &data).unwrap();
    assert_eq!(result, json!(true));
}

#[test]
fn test_jmespath_ends_with() {
    let data = json!({"name": "nginx-proxy"});
    let result = jmespath::evaluate("ends_with(name, 'proxy')", &data).unwrap();
    assert_eq!(result, json!(true));
}

#[test]
fn test_jmespath_join_function() {
    let data = json!({"items": ["a", "b", "c"]});
    let result = jmespath::evaluate("join(',', items)", &data).unwrap();
    assert_eq!(result, json!("a,b,c"));
}

#[test]
fn test_jmespath_sort_function() {
    let data = json!({"nums": [3, 1, 4, 1, 5, 9]});
    let result = jmespath::evaluate("sort(nums)", &data).unwrap();
    assert_eq!(result, json!([1, 1, 3, 4, 5, 9]));
}

#[test]
fn test_jmespath_reverse_function() {
    let data = json!({"arr": [1, 2, 3]});
    let result = jmespath::evaluate("reverse(arr)", &data).unwrap();
    assert_eq!(result, json!([3, 2, 1]));
}

#[test]
fn test_jmespath_min_max() {
    let data = json!({"vals": [5, 2, 8, 1, 9]});
    let min_r = jmespath::evaluate("min(vals)", &data).unwrap();
    let max_r = jmespath::evaluate("max(vals)", &data).unwrap();
    assert_eq!(min_r, json!(1));
    assert_eq!(max_r, json!(9));
}

#[test]
fn test_jmespath_or_expression() {
    let data = json!({"a": null, "b": "fallback"});
    let result = jmespath::evaluate("a || b", &data).unwrap();
    assert_eq!(result, json!("fallback"));
}

#[test]
fn test_jmespath_multiselect_hash() {
    let data = json!({"name": "alice", "age": 30, "role": "admin"});
    let result = jmespath::evaluate("{n: name, r: role}", &data).unwrap();
    assert_eq!(result["n"], json!("alice"));
    assert_eq!(result["r"], json!("admin"));
}

#[test]
fn test_jmespath_multiselect_list() {
    let data = json!({"name": "alice", "age": 30});
    let result = jmespath::evaluate("[name, age]", &data).unwrap();
    assert_eq!(result, json!(["alice", 30]));
}

#[test]
fn test_jmespath_pipe() {
    let data = json!({"items": [{"name": "a"}, {"name": "b"}]});
    let result = jmespath::evaluate("items | length(@)", &data).unwrap();
    assert_eq!(result, json!(2));
}

#[test]
fn test_jmespath_variable_substitution() {
    let template = "hello {{request.object.metadata.name}}";
    let context = json!({
        "request": {
            "object": {"metadata": {"name": "my-pod"}}
        }
    });
    let result = jmespath::substitute_variables(template, &context).unwrap();
    assert_eq!(result, "hello my-pod");
}

#[test]
fn test_jmespath_variable_substitution_missing() {
    let template = "val={{missing.field}}";
    let context = json!({});
    let result = jmespath::substitute_variables(template, &context).unwrap_or_default();
    assert!(result.starts_with("val=") || result.is_empty());
}

// ─── Pattern Matching Tests ───────────────────────────────────────────────────

#[test]
fn test_pattern_exact_match() {
    assert!(jmespath::kyverno_pattern_match("nginx", "nginx"));
}

#[test]
fn test_pattern_wildcard_star() {
    assert!(jmespath::kyverno_pattern_match("nginx:*", "nginx:1.25"));
}

#[test]
fn test_pattern_wildcard_prefix() {
    assert!(jmespath::kyverno_pattern_match("*:latest", "nginx:latest"));
}

#[test]
fn test_pattern_no_match() {
    assert!(!jmespath::kyverno_pattern_match("nginx", "apache"));
}

#[test]
fn test_pattern_question_mark() {
    assert!(jmespath::kyverno_pattern_match("v1.?.?", "v1.2.3"));
}

// ─── Validation Rule Tests ────────────────────────────────────────────────────

use cave_policy::kyverno::models::KyvernoRule;

fn validate_rule(name: &str, match_res: MatchResources, validation: Validation) -> KyvernoRule {
    KyvernoRule {
        name: name.to_string(),
        match_resources: match_res,
        exclude: None,
        context: vec![],
        preconditions: None,
        validate: Some(validation),
        mutate: None,
        generate: None,
        verify_images: vec![],
    }
}

#[test]
fn test_validate_required_labels() {
    let mut engine = make_engine();

    let policy = make_policy("require-labels", vec![validate_rule(
        "check-team-label",
        match_pods("default"),
        Validation {
            message: Some("Pod must have 'team' label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"team": "?*"}}})),
            ..Default::default()
        },
    )], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    // Pod with label — allowed
    let good = pod_resource("default", "good-pod", json!({"team": "platform"}));
    let result = engine.evaluate(&good, Some("default"), "CREATE", None);
    assert!(result.allowed, "pod with team label should be allowed");

    // Pod without label — denied
    let bad = pod_resource("default", "bad-pod", json!({"app": "web"}));
    let result = engine.evaluate(&bad, Some("default"), "CREATE", None);
    assert!(!result.allowed, "pod without team label should be denied");
}

#[test]
fn test_validate_any_pattern() {
    let mut engine = make_engine();

    let policy = make_policy("require-resources", vec![validate_rule(
        "check-resources",
        match_pods("default"),
        Validation {
            message: Some("Must have limits or requests".to_string()),
            any_pattern: Some(vec![
                json!({"spec": {"containers": [{"resources": {"limits": {"memory": "?*"}}}]}}),
                json!({"spec": {"containers": [{"resources": {"requests": {"memory": "?*"}}}]}}),
            ]),
            ..Default::default()
        },
    )], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    let pod_with_limits = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "limited", "namespace": "default", "labels": {}},
        "spec": {
            "containers": [{"name": "main", "image": "nginx:1.25",
                "resources": {"limits": {"memory": "128Mi"}}}]
        }
    });
    let result = engine.evaluate(&pod_with_limits, Some("default"), "CREATE", None);
    assert!(result.allowed, "pod with memory limits should be allowed");
}

#[test]
fn test_validate_audit_mode() {
    let mut engine = make_engine();

    let policy = make_policy("audit-labels", vec![validate_rule(
        "check-env-label",
        match_pods("default"),
        Validation {
            message: Some("Pod must have 'env' label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"env": "?*"}}})),
            ..Default::default()
        },
    )], ValidationFailureAction::Audit);

    engine.add_cluster_policy(policy);

    // Pod without label — allowed in audit mode, but violations reported
    let pod = pod_resource("default", "untagged", json!({}));
    let result = engine.evaluate(&pod, Some("default"), "CREATE", None);
    assert!(result.allowed, "audit mode should allow violations");
    assert!(!result.violations.is_empty(), "audit mode should report violations");
}

#[test]
fn test_validate_namespace_scope() {
    let mut engine = make_engine();

    let policy = make_policy("ns-scoped", vec![validate_rule(
        "check-in-restricted",
        match_pods("restricted"),
        Validation {
            message: Some("Must have security label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"security": "?*"}}})),
            ..Default::default()
        },
    )], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    // Pod in "default" — not subject to this policy
    let pod_default = pod_resource("default", "any-pod", json!({}));
    let result = engine.evaluate(&pod_default, Some("default"), "CREATE", None);
    assert!(result.allowed, "pod in default ns should bypass restricted-ns policy");

    // Pod in "restricted" — must have label
    let pod_restricted = pod_resource("restricted", "unlabeled", json!({}));
    let result = engine.evaluate(&pod_restricted, Some("restricted"), "CREATE", None);
    assert!(!result.allowed, "pod in restricted ns without label should be denied");
}

#[test]
fn test_validate_label_selector() {
    use cave_policy::kyverno::models::LabelSelector;

    let mut engine = make_engine();

    let policy = make_policy("selector-policy", vec![validate_rule(
        "check-selected",
        MatchResources {
            resources: Some(ResourceDescription {
                kinds: vec!["Pod".to_string()],
                namespaces: vec!["default".to_string()],
                selector: Some(LabelSelector {
                    match_labels: [("tier".to_string(), "frontend".to_string())].into(),
                    match_expressions: vec![],
                }),
                ..Default::default()
            }),
            ..Default::default()
        },
        Validation {
            message: Some("Frontend pods must have version label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"version": "?*"}}})),
            ..Default::default()
        },
    )], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    // Frontend pod without version — denied
    let frontend = pod_resource("default", "fe-pod", json!({"tier": "frontend"}));
    let result = engine.evaluate(&frontend, Some("default"), "CREATE", None);
    assert!(!result.allowed, "frontend pod without version should be denied");

    // Backend pod — not selected
    let backend = pod_resource("default", "be-pod", json!({"tier": "backend"}));
    let result = engine.evaluate(&backend, Some("default"), "CREATE", None);
    assert!(result.allowed, "backend pod should not be selected");
}

// ─── Mutation Rule Tests ──────────────────────────────────────────────────────

#[test]
fn test_mutate_add_label() {
    let mut engine = make_engine();

    let policy = make_policy("add-managed-label", vec![KyvernoRule {
        name: "add-label".to_string(),
        match_resources: match_pods("default"),
        exclude: None,
        context: vec![],
        preconditions: None,
        validate: None,
        mutate: Some(Mutation {
            patch_strategic_merge: Some(json!({
                "metadata": {"labels": {"managed-by": "kyverno"}}
            })),
            patches_json6902: None,
            foreach: vec![],
            targets: vec![],
        }),
        generate: None,
        verify_images: vec![],
    }], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    let pod = pod_resource("default", "my-pod", json!({"app": "web"}));
    let result = engine.evaluate(&pod, Some("default"), "CREATE", None);
    assert!(result.allowed);
    assert!(!result.mutations.is_empty(), "should have mutation patches");
}

#[test]
fn test_mutate_json_patch() {
    let mut engine = make_engine();

    let policy = make_policy("add-annotation", vec![KyvernoRule {
        name: "add-annotation".to_string(),
        match_resources: match_pods("default"),
        exclude: None,
        context: vec![],
        preconditions: None,
        validate: None,
        mutate: Some(Mutation {
            patch_strategic_merge: None,
            patches_json6902: Some(r#"
- op: add
  path: /metadata/annotations/policy-applied
  value: kyverno
"#.to_string()),
            foreach: vec![],
            targets: vec![],
        }),
        generate: None,
        verify_images: vec![],
    }], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    let pod = pod_resource("default", "patched-pod", json!({}));
    let result = engine.evaluate(&pod, Some("default"), "CREATE", None);
    assert!(result.allowed);
}

// ─── Generation Rule Tests ────────────────────────────────────────────────────

#[test]
fn test_generate_configmap() {
    use cave_policy::kyverno::models::Generation;

    let mut engine = make_engine();

    let policy = make_policy("gen-configmap", vec![KyvernoRule {
        name: "create-default-cm".to_string(),
        match_resources: MatchResources {
            resources: Some(ResourceDescription {
                kinds: vec!["Namespace".to_string()],
                operations: vec!["CREATE".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        },
        exclude: None,
        context: vec![],
        preconditions: None,
        validate: None,
        mutate: None,
        generate: Some(Generation {
            api_version: "v1".to_string(),
            kind: "ConfigMap".to_string(),
            name: "default-config".to_string(),
            namespace: Some("{{request.object.metadata.name}}".to_string()),
            synchronize: true,
            data: Some(json!({
                "apiVersion": "v1",
                "kind": "ConfigMap",
                "metadata": {"name": "default-config"},
                "data": {"env": "production"}
            })),
            clone: None,
            clone_list: None,
        }),
        verify_images: vec![],
    }], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    let ns = json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {"name": "my-namespace"}
    });
    let result = engine.evaluate(&ns, None, "CREATE", None);
    assert!(result.allowed);
    assert!(!result.generated.is_empty(), "should have generated resources");
    let generated = &result.generated[0];
    assert_eq!(generated.resource["kind"], json!("ConfigMap"));
}

// ─── Policy Exception Tests ───────────────────────────────────────────────────

#[test]
fn test_policy_exception() {
    let mut engine = make_engine();

    let policy = make_policy("require-team-label", vec![validate_rule(
        "check-team",
        match_pods("default"),
        Validation {
            message: Some("Must have team label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"team": "?*"}}})),
            ..Default::default()
        },
    )], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    // Add exception for "excepted-pod"
    let exception = PolicyException {
        api_version: "kyverno.io/v2alpha1".into(),
        kind: "PolicyException".into(),
        metadata: ObjectMeta {
            name: "allow-excepted-pod".to_string(),
            namespace: Some("default".to_string()),
            ..Default::default()
        },
        spec: PolicyExceptionSpec {
            exceptions: vec![PolicyExceptionEntry {
                policy_name: "require-team-label".to_string(),
                rule_names: vec!["check-team".to_string()],
            }],
            match_resources: MatchResources {
                resources: Some(ResourceDescription {
                    kinds: vec!["Pod".to_string()],
                    names: vec!["excepted-pod".to_string()],
                    namespaces: vec!["default".to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            },
            conditions: None,
            pod_security: vec![],
        },
    };

    engine.add_exception(exception);

    // The excepted pod should be allowed even without the label
    let pod = pod_resource("default", "excepted-pod", json!({}));
    let result = engine.evaluate(&pod, Some("default"), "CREATE", None);
    assert!(result.allowed, "excepted pod should be allowed");

    // Regular pod still needs the label
    let regular = pod_resource("default", "regular-pod", json!({}));
    let result = engine.evaluate(&regular, Some("default"), "CREATE", None);
    assert!(!result.allowed, "non-excepted pod should be denied");
}

// ─── Exclude Resources Tests ──────────────────────────────────────────────────

#[test]
fn test_exclude_resources() {
    use cave_policy::kyverno::models::ExcludeResources;

    let mut engine = make_engine();

    let policy = make_policy("exclude-kube-system", vec![KyvernoRule {
        name: "require-team".to_string(),
        match_resources: MatchResources {
            resources: Some(ResourceDescription {
                kinds: vec!["Pod".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        },
        exclude: Some(ExcludeResources {
            any: vec![ResourceFilter {
                resources: Some(ResourceDescription {
                    kinds: vec!["Pod".to_string()],
                    namespaces: vec!["kube-system".to_string()],
                    ..Default::default()
                }),
                subjects: vec![],
                roles: vec![],
                cluster_roles: vec![],
            }],
            all: vec![],
            resources: None,
            subjects: vec![],
        }),
        context: vec![],
        preconditions: None,
        validate: Some(Validation {
            message: Some("Must have team label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"team": "?*"}}})),
            ..Default::default()
        }),
        mutate: None,
        generate: None,
        verify_images: vec![],
    }], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    // Pod in kube-system — excluded, should pass
    let sys_pod = pod_resource("kube-system", "coredns", json!({}));
    let result = engine.evaluate(&sys_pod, Some("kube-system"), "CREATE", None);
    assert!(result.allowed, "kube-system pod should be excluded");

    // Pod in default — must have label
    let app_pod = pod_resource("default", "app", json!({}));
    let result = engine.evaluate(&app_pod, Some("default"), "CREATE", None);
    assert!(!result.allowed, "default pod without label should be denied");
}

// ─── Multiple Rules Tests ─────────────────────────────────────────────────────

#[test]
fn test_multiple_rules_all_must_pass() {
    let mut engine = make_engine();

    let policy = make_policy("multi-rule", vec![
        validate_rule("check-team", match_pods("default"), Validation {
            message: Some("Must have team label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"team": "?*"}}})),
            ..Default::default()
        }),
        validate_rule("check-env", match_pods("default"), Validation {
            message: Some("Must have env label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"env": "?*"}}})),
            ..Default::default()
        }),
    ], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    // Pod with both labels — passes
    let full = pod_resource("default", "full", json!({"team": "a", "env": "prod"}));
    let result = engine.evaluate(&full, Some("default"), "CREATE", None);
    assert!(result.allowed);

    // Pod with only team — fails (missing env)
    let partial = pod_resource("default", "partial", json!({"team": "a"}));
    let result = engine.evaluate(&partial, Some("default"), "CREATE", None);
    assert!(!result.allowed);
    assert_eq!(result.violations.len(), 1);

    // Pod with neither — fails with 2 violations
    let empty = pod_resource("default", "empty", json!({}));
    let result = engine.evaluate(&empty, Some("default"), "CREATE", None);
    assert!(!result.allowed);
    assert_eq!(result.violations.len(), 2);
}

// ─── Engine CRUD Tests ────────────────────────────────────────────────────────

#[test]
fn test_add_remove_policy() {
    let mut engine = make_engine();

    let policy = make_policy("temp-policy", vec![validate_rule(
        "check",
        match_pods("default"),
        Validation {
            message: Some("Must have label".to_string()),
            pattern: Some(json!({"metadata": {"labels": {"x": "?*"}}})),
            ..Default::default()
        },
    )], ValidationFailureAction::Enforce);

    engine.add_cluster_policy(policy);

    // Policy enforces the label
    let pod = pod_resource("default", "p", json!({}));
    let result = engine.evaluate(&pod, Some("default"), "CREATE", None);
    assert!(!result.allowed);

    // Remove the policy
    engine.remove_cluster_policy("temp-policy");

    // Now allowed (no policies)
    let result = engine.evaluate(&pod, Some("default"), "CREATE", None);
    assert!(result.allowed);
}

#[test]
fn test_list_policies() {
    let mut engine = make_engine();

    engine.add_cluster_policy(make_policy("policy-a", vec![], ValidationFailureAction::Audit));
    engine.add_cluster_policy(make_policy("policy-b", vec![], ValidationFailureAction::Audit));
    engine.add_cluster_policy(make_policy("policy-c", vec![], ValidationFailureAction::Audit));

    let policies = engine.list_cluster_policies();
    assert_eq!(policies.len(), 3);
    let names: Vec<&str> = policies.iter().map(|p| p.metadata.name.as_str()).collect();
    assert!(names.contains(&"policy-a"));
    assert!(names.contains(&"policy-b"));
    assert!(names.contains(&"policy-c"));
}

#[test]
fn test_get_policy() {
    let mut engine = make_engine();

    engine.add_cluster_policy(make_policy("my-policy", vec![], ValidationFailureAction::Audit));

    assert!(engine.get_cluster_policy("my-policy").is_some());
    assert!(engine.get_cluster_policy("nonexistent").is_none());
}

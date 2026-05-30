// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Kyverno PolicyException condition evaluation.
//!
//! Upstream: kyverno/kyverno v1.18.1 — pkg/engine/handlers/exceptions +
//! api/kyverno/v2 PolicyException. An exception applies only when the resource
//! matches the exception's `match` block AND the exception's `conditions`
//! (any/all) evaluate true. The pre-existing `has_exception` honored name +
//! match scoping but silently ignored `conditions`, so a conditioned exception
//! leaked onto every matched resource.

use cave_policy::kyverno::{
    KyvernoEngine,
    models::{
        ClusterPolicy, Condition, ConditionOperator, Conditions, MatchResources, ObjectMeta,
        PolicyException, PolicyExceptionEntry, PolicyExceptionSpec, PolicySpec, ResourceDescription,
        Validation, ValidationFailureAction,
    },
};
use serde_json::json;

fn require_team_policy() -> ClusterPolicy {
    ClusterPolicy {
        api_version: "kyverno.io/v1".into(),
        kind: "ClusterPolicy".into(),
        metadata: ObjectMeta {
            name: "require-team-label".into(),
            ..Default::default()
        },
        spec: PolicySpec {
            validation_failure_action: ValidationFailureAction::Enforce,
            rules: vec![cave_policy::kyverno::models::KyvernoRule {
                name: "check-team".into(),
                match_resources: MatchResources {
                    resources: Some(ResourceDescription {
                        kinds: vec!["Pod".into()],
                        namespaces: vec!["default".into()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                exclude: None,
                context: vec![],
                preconditions: None,
                validate: Some(Validation {
                    message: Some("Must have team label".into()),
                    pattern: Some(json!({"metadata": {"labels": {"team": "?*"}}})),
                    ..Default::default()
                }),
                mutate: None,
                generate: None,
                verify_images: vec![],
            }],
            ..Default::default()
        },
        status: None,
    }
}

/// Exception scoped to all Pods in `default`, gated on `env == prod`.
fn conditioned_exception() -> PolicyException {
    PolicyException {
        api_version: "kyverno.io/v2".into(),
        kind: "PolicyException".into(),
        metadata: ObjectMeta {
            name: "prod-only-exception".into(),
            namespace: Some("default".into()),
            ..Default::default()
        },
        spec: PolicyExceptionSpec {
            exceptions: vec![PolicyExceptionEntry {
                policy_name: "require-team-label".into(),
                rule_names: vec!["check-team".into()],
            }],
            match_resources: MatchResources {
                resources: Some(ResourceDescription {
                    kinds: vec!["Pod".into()],
                    namespaces: vec!["default".into()],
                    ..Default::default()
                }),
                ..Default::default()
            },
            conditions: Some(Conditions {
                all: Some(vec![Condition {
                    key: json!("{{ request.object.metadata.labels.env }}"),
                    operator: ConditionOperator::Equals,
                    value: Some(json!("prod")),
                    message: None,
                }]),
                any: None,
            }),
            pod_security: vec![],
        },
    }
}

fn pod(name: &str, env: &str) -> serde_json::Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": name, "namespace": "default", "labels": {"env": env}},
        "spec": {"containers": [{"name": "main", "image": "nginx:1.25"}]}
    })
}

#[test]
fn test_exception_condition_satisfied_applies() {
    let mut engine = KyvernoEngine::new();
    engine.add_cluster_policy(require_team_policy());
    engine.add_exception(conditioned_exception());

    // env == prod → condition true → exception applies → allowed despite missing team label.
    let p = pod("prod-pod", "prod");
    let result = engine.evaluate(&p, Some("default"), "CREATE", None);
    assert!(
        result.allowed,
        "prod pod must inherit the exception (condition satisfied)"
    );
}

#[test]
fn test_exception_condition_unsatisfied_does_not_apply() {
    let mut engine = KyvernoEngine::new();
    engine.add_cluster_policy(require_team_policy());
    engine.add_exception(conditioned_exception());

    // env == dev → condition false → exception must NOT apply → denied (no team label).
    let p = pod("dev-pod", "dev");
    let result = engine.evaluate(&p, Some("default"), "CREATE", None);
    assert!(
        !result.allowed,
        "dev pod must NOT inherit the prod-gated exception"
    );
}

#[test]
fn test_exception_no_conditions_still_applies() {
    // Regression: an exception without conditions keeps its previous behavior.
    let mut engine = KyvernoEngine::new();
    engine.add_cluster_policy(require_team_policy());
    let mut exc = conditioned_exception();
    exc.spec.conditions = None;
    engine.add_exception(exc);

    let p = pod("any-pod", "dev");
    let result = engine.evaluate(&p, Some("default"), "CREATE", None);
    assert!(
        result.allowed,
        "unconditioned exception applies to every matched resource"
    );
}

#[test]
fn test_exception_condition_any_branch() {
    // `any` semantics: at least one condition must hold.
    let mut engine = KyvernoEngine::new();
    engine.add_cluster_policy(require_team_policy());
    let mut exc = conditioned_exception();
    exc.spec.conditions = Some(Conditions {
        any: Some(vec![
            Condition {
                key: json!("{{ request.object.metadata.labels.env }}"),
                operator: ConditionOperator::Equals,
                value: Some(json!("staging")),
                message: None,
            },
            Condition {
                key: json!("{{ request.object.metadata.labels.env }}"),
                operator: ConditionOperator::Equals,
                value: Some(json!("prod")),
                message: None,
            },
        ]),
        all: None,
    });
    engine.add_exception(exc);

    let p = pod("prod-pod", "prod");
    let result = engine.evaluate(&p, Some("default"), "CREATE", None);
    assert!(result.allowed, "any-branch matches prod → exception applies");
}

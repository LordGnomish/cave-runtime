// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for cave_admission::evaluator (PolicyEvaluator).
//! These tests assert the OBSERVED behaviour of pre-existing code.

use cave_admission::evaluator::PolicyEvaluator;
use cave_admission::models::{
    AdmissionOperation, AdmissionPolicy, EnforcementAction, GroupVersionResource, PolicyEvalRule,
    RuleOperator, UserInfo,
};
use cave_admission::models::AdmissionRequest;
use chrono::Utc;
use uuid::Uuid;

fn make_request(obj: serde_json::Value) -> AdmissionRequest {
    AdmissionRequest {
        uid: Uuid::new_v4(),
        operation: AdmissionOperation::Create,
        resource: GroupVersionResource {
            group: "".into(),
            version: "v1".into(),
            resource: "pods".into(),
        },
        object: obj,
        old_object: None,
        user_info: UserInfo {
            username: "test-user".into(),
            uid: "uid-1".into(),
            groups: vec!["system:masters".into()],
        },
        dry_run: false,
    }
}

fn make_policy(rules: Vec<PolicyEvalRule>, action: EnforcementAction) -> AdmissionPolicy {
    AdmissionPolicy {
        id: Uuid::new_v4(),
        name: "char-policy".into(),
        description: "characterization test policy".into(),
        operation_types: vec![AdmissionOperation::Create],
        resource_types: vec!["pods".into()],
        enforcement_action: action,
        rules,
        enabled: true,
        created_at: Utc::now(),
    }
}

// --- EnforcementAction::Deny blocks the request ---------------------------------

#[test]
fn deny_action_blocks_when_rule_violated() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({
        "metadata": {"name": "bad-pod", "namespace": "default"},
        "spec": {"hostNetwork": true}
    });
    let request = make_request(obj);
    let policy = make_policy(
        vec![PolicyEvalRule {
            field: "spec.hostNetwork".into(),
            operator: RuleOperator::Equals,
            value: serde_json::json!(false),
        }],
        EnforcementAction::Deny,
    );
    let (response, logs) = evaluator.evaluate(&request, &[policy]);
    assert!(!response.allowed, "Deny policy should block the request");
    assert!(!logs.is_empty(), "A violation log should be recorded");
    assert_eq!(response.status.as_ref().unwrap().code, 403);
}

// --- EnforcementAction::Warn allows but attaches a warning ----------------------

#[test]
fn warn_action_allows_with_warning() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({
        "metadata": {"name": "warn-pod", "namespace": "default"},
        "spec": {}
    });
    let request = make_request(obj);
    let policy = make_policy(
        vec![PolicyEvalRule {
            field: "spec.resources".into(),
            operator: RuleOperator::Exists,
            value: serde_json::json!(null),
        }],
        EnforcementAction::Warn,
    );
    let (response, _logs) = evaluator.evaluate(&request, &[policy]);
    assert!(response.allowed, "Warn policy should allow the request");
    assert!(!response.warnings.is_empty(), "A warning should be attached");
}

// --- EnforcementAction::Audit allows silently -----------------------------------

#[test]
fn audit_action_allows_silently() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({
        "metadata": {"name": "audit-pod", "namespace": "default"},
        "spec": {}
    });
    let request = make_request(obj);
    let policy = make_policy(
        vec![PolicyEvalRule {
            field: "spec.resources".into(),
            operator: RuleOperator::Exists,
            value: serde_json::json!(null),
        }],
        EnforcementAction::Audit,
    );
    let (response, logs) = evaluator.evaluate(&request, &[policy]);
    assert!(response.allowed, "Audit policy should allow the request");
    assert!(response.warnings.is_empty(), "No warnings for audit mode");
    assert!(!logs.is_empty(), "A violation log should still be recorded");
}

// --- evaluate_mutating injects the cave-admission patch -------------------------

#[test]
fn mutating_injects_cave_admission_patch() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"metadata": {"name": "ok-pod", "namespace": "default"}, "spec": {}});
    let request = make_request(obj);
    let (response, _logs) = evaluator.evaluate_mutating(&request, &[]);
    assert!(response.allowed);
    assert!(response.patch.is_some(), "Patch must be set for mutating webhook");
    assert_eq!(
        response.patch_type.as_deref(),
        Some("JSONPatch"),
        "patch_type must be JSONPatch"
    );
}

// --- Disabled policy is skipped -------------------------------------------------

#[test]
fn disabled_policy_is_skipped() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"metadata": {"name": "p", "namespace": "default"}, "spec": {"hostNetwork": true}});
    let request = make_request(obj);
    let mut policy = make_policy(
        vec![PolicyEvalRule {
            field: "spec.hostNetwork".into(),
            operator: RuleOperator::Equals,
            value: serde_json::json!(false),
        }],
        EnforcementAction::Deny,
    );
    policy.enabled = false;
    let (response, logs) = evaluator.evaluate(&request, &[policy]);
    assert!(response.allowed, "Disabled policy must be skipped");
    assert!(logs.is_empty());
}

// --- Operation filter -----------------------------------------------------------

#[test]
fn policy_skipped_when_operation_does_not_match() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"metadata": {"name": "p"}, "spec": {"hostNetwork": true}});
    // policy only fires on Update
    let policy = AdmissionPolicy {
        id: Uuid::new_v4(),
        name: "update-only".into(),
        description: "".into(),
        operation_types: vec![AdmissionOperation::Update],
        resource_types: vec!["pods".into()],
        enforcement_action: EnforcementAction::Deny,
        rules: vec![PolicyEvalRule {
            field: "spec.hostNetwork".into(),
            operator: RuleOperator::Equals,
            value: serde_json::json!(false),
        }],
        enabled: true,
        created_at: Utc::now(),
    };
    // request is Create
    let request = make_request(obj);
    let (response, logs) = evaluator.evaluate(&request, &[policy]);
    assert!(response.allowed, "Policy should be skipped — operation mismatch");
    assert!(logs.is_empty());
}

// --- extract_field dot-notation -------------------------------------------------

#[test]
fn extract_field_handles_dot_notation() {
    let obj = serde_json::json!({"spec": {"securityContext": {"privileged": true}}});
    let val = PolicyEvaluator::extract_field(&obj, "spec.securityContext.privileged");
    assert_eq!(val, Some(&serde_json::json!(true)));
}

#[test]
fn extract_field_returns_none_for_missing_path() {
    let obj = serde_json::json!({"spec": {}});
    let val = PolicyEvaluator::extract_field(&obj, "spec.missing.nested");
    assert!(val.is_none());
}

// --- RuleOperator variants ------------------------------------------------------

#[test]
fn greater_than_operator() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"spec": {"replicas": 5}});
    let rule = PolicyEvalRule {
        field: "spec.replicas".into(),
        operator: RuleOperator::GreaterThan,
        value: serde_json::json!(3),
    };
    assert!(evaluator.evaluate_rule(&obj, &rule).is_none(), "5 > 3 should pass");
    let rule2 = PolicyEvalRule {
        field: "spec.replicas".into(),
        operator: RuleOperator::GreaterThan,
        value: serde_json::json!(10),
    };
    assert!(evaluator.evaluate_rule(&obj, &rule2).is_some(), "5 > 10 should fail");
}

#[test]
fn less_than_operator() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"spec": {"replicas": 2}});
    let rule = PolicyEvalRule {
        field: "spec.replicas".into(),
        operator: RuleOperator::LessThan,
        value: serde_json::json!(5),
    };
    assert!(evaluator.evaluate_rule(&obj, &rule).is_none(), "2 < 5 should pass");
}

#[test]
fn not_exists_operator_rejects_present_field() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"spec": {"hostPID": true}});
    let rule = PolicyEvalRule {
        field: "spec.hostPID".into(),
        operator: RuleOperator::NotExists,
        value: serde_json::json!(null),
    };
    assert!(evaluator.evaluate_rule(&obj, &rule).is_some(), "Field exists — rule should fail");
}

#[test]
fn contains_operator_works_for_strings() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"spec": {"image": "docker.io/nginx:latest"}});
    let rule = PolicyEvalRule {
        field: "spec.image".into(),
        operator: RuleOperator::Contains,
        value: serde_json::json!(":latest"),
    };
    assert!(evaluator.evaluate_rule(&obj, &rule).is_none(), "Image contains :latest — rule should pass");
}

#[test]
fn not_contains_blocks_latest_tag() {
    let evaluator = PolicyEvaluator::new();
    let obj = serde_json::json!({"spec": {"image": "docker.io/nginx:latest"}});
    let rule = PolicyEvalRule {
        field: "spec.image".into(),
        operator: RuleOperator::NotContains,
        value: serde_json::json!(":latest"),
    };
    assert!(
        evaluator.evaluate_rule(&obj, &rule).is_some(),
        "NotContains(:latest) should fail for :latest image"
    );
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for cave_admission::store (AdmissionStore).

use cave_admission::models::{
    AdmissionOperation, AdmissionPolicy, EnforcementAction, PolicyEvalRule, RuleOperator,
    ViolationLog, AdmissionRequest, GroupVersionResource, UserInfo,
};
use cave_admission::store::AdmissionStore;
use chrono::Utc;
use uuid::Uuid;

fn make_policy(name: &str) -> AdmissionPolicy {
    AdmissionPolicy {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: "".into(),
        operation_types: vec![AdmissionOperation::Create],
        resource_types: vec!["pods".into()],
        enforcement_action: EnforcementAction::Deny,
        rules: vec![PolicyEvalRule {
            field: "spec.hostNetwork".into(),
            operator: RuleOperator::NotEquals,
            value: serde_json::json!(true),
        }],
        enabled: true,
        created_at: Utc::now(),
    }
}

fn make_violation(policy_id: Uuid, request_uid: Uuid) -> ViolationLog {
    ViolationLog {
        id: Uuid::new_v4(),
        policy_id,
        request_uid,
        resource_name: "test-pod".into(),
        resource_namespace: "default".into(),
        operation: AdmissionOperation::Create,
        message: "test violation".into(),
        enforcement_action: EnforcementAction::Deny,
        logged_at: Utc::now(),
    }
}

fn make_request() -> AdmissionRequest {
    AdmissionRequest {
        uid: Uuid::new_v4(),
        operation: AdmissionOperation::Create,
        resource: GroupVersionResource {
            group: "".into(),
            version: "v1".into(),
            resource: "pods".into(),
        },
        object: serde_json::json!({
            "metadata": {"name": "test-pod", "namespace": "default"}
        }),
        old_object: None,
        user_info: UserInfo {
            username: "test-user".into(),
            uid: "uid-1".into(),
            groups: vec![],
        },
        dry_run: false,
    }
}

// --- CRUD -----------------------------------------------------------------------

#[test]
fn insert_and_get_policy() {
    let store = AdmissionStore::new();
    let p = make_policy("test-policy");
    let id = p.id;
    store.insert_policy(p.clone());
    let fetched = store.get_policy(id).expect("policy should be retrievable");
    assert_eq!(fetched.name, "test-policy");
}

#[test]
fn list_policies_returns_all_sorted_by_name() {
    let store = AdmissionStore::new();
    store.insert_policy(make_policy("beta-policy"));
    store.insert_policy(make_policy("alpha-policy"));
    let list = store.list_policies();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, "alpha-policy");
    assert_eq!(list[1].name, "beta-policy");
}

#[test]
fn update_policy_changes_stored_record() {
    let store = AdmissionStore::new();
    let p = make_policy("original");
    let id = p.id;
    store.insert_policy(p);
    let mut updated = make_policy("updated");
    updated.id = id;
    store.update_policy(id, updated.clone());
    let fetched = store.get_policy(id).unwrap();
    assert_eq!(fetched.name, "updated");
}

#[test]
fn update_nonexistent_policy_returns_none() {
    let store = AdmissionStore::new();
    let result = store.update_policy(Uuid::new_v4(), make_policy("ghost"));
    assert!(result.is_none());
}

#[test]
fn delete_policy_removes_it() {
    let store = AdmissionStore::new();
    let p = make_policy("to-delete");
    let id = p.id;
    store.insert_policy(p);
    assert!(store.delete_policy(id));
    assert!(store.get_policy(id).is_none());
}

#[test]
fn delete_nonexistent_returns_false() {
    let store = AdmissionStore::new();
    assert!(!store.delete_policy(Uuid::new_v4()));
}

#[test]
fn set_policy_enabled_false() {
    let store = AdmissionStore::new();
    let p = make_policy("enable-test");
    let id = p.id;
    store.insert_policy(p);
    let updated = store.set_policy_enabled(id, false).unwrap();
    assert!(!updated.enabled);
    assert!(!store.get_policy(id).unwrap().enabled);
}

// --- Violations -----------------------------------------------------------------

#[test]
fn log_violation_and_retrieve_recent() {
    let store = AdmissionStore::new();
    let v = make_violation(Uuid::new_v4(), Uuid::new_v4());
    store.log_violation(v.clone());
    let recent = store.recent_violations(10);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].resource_name, "test-pod");
}

#[test]
fn recent_violations_respects_limit() {
    let store = AdmissionStore::new();
    for _ in 0..5 {
        store.log_violation(make_violation(Uuid::new_v4(), Uuid::new_v4()));
    }
    let recent = store.recent_violations(3);
    assert_eq!(recent.len(), 3, "Should return only the last 3");
}

#[test]
fn violations_by_policy_filters_correctly() {
    let store = AdmissionStore::new();
    let pid1 = Uuid::new_v4();
    let pid2 = Uuid::new_v4();
    store.log_violation(make_violation(pid1, Uuid::new_v4()));
    store.log_violation(make_violation(pid2, Uuid::new_v4()));
    store.log_violation(make_violation(pid1, Uuid::new_v4()));
    assert_eq!(store.violations_by_policy(pid1).len(), 2);
    assert_eq!(store.violations_by_policy(pid2).len(), 1);
}

#[test]
fn log_violation_for_request_extracts_resource_name() {
    let store = AdmissionStore::new();
    let request = make_request();
    let policy_id = Uuid::new_v4();
    store.log_violation_for_request(
        policy_id,
        &request,
        "hostNetwork=true is not allowed",
        EnforcementAction::Deny,
    );
    let violations = store.violations_by_policy(policy_id);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].resource_name, "test-pod");
    assert_eq!(violations[0].resource_namespace, "default");
}

// --- Stats ----------------------------------------------------------------------

#[test]
fn stats_counts_policies_and_violations() {
    let store = AdmissionStore::new();
    store.insert_policy(make_policy("p1"));
    store.insert_policy(make_policy("p2"));
    store.log_violation(make_violation(Uuid::new_v4(), Uuid::new_v4()));
    let stats = store.stats();
    assert_eq!(stats.total_policies, 2);
    assert_eq!(stats.enabled_policies, 2);
    assert_eq!(stats.total_violations, 1);
}

// --- Seed defaults --------------------------------------------------------------

#[test]
fn seed_default_policies_loads_at_least_five_policies() {
    let store = AdmissionStore::new();
    store.seed_default_policies();
    let policies = store.list_policies();
    assert!(
        policies.len() >= 5,
        "Expected >= 5 default policies, got {}",
        policies.len()
    );
    assert!(policies.iter().all(|p| p.enabled));
}

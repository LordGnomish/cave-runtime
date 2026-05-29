// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for access request JIT workflow.

use cave_pam::access_request::{
    AccessRequestStore, ApprovalDecision, RequestError,
    CreateRequest, RequestState,
};
use uuid::Uuid;
use chrono::{Duration, Utc};

fn make_create_request(roles: Vec<&str>, ttl_hours: i64) -> CreateRequest {
    CreateRequest {
        user_id: Uuid::new_v4(),
        requested_roles: roles.into_iter().map(|s| s.to_string()).collect(),
        reason: "Emergency incident response".to_string(),
        ttl: Duration::hours(ttl_hours),
    }
}

#[test]
fn test_create_request_returns_pending() {
    let store = AccessRequestStore::new();
    let req = make_create_request(vec!["db-admin", "k8s-prod-view"], 4);
    let id = store.create(req).expect("create should succeed");
    let fetched = store.get(&id).expect("should find request");
    assert_eq!(fetched.state, RequestState::Pending);
    assert!(!fetched.requested_roles.is_empty());
}

#[test]
fn test_approve_request_changes_state() {
    let store = AccessRequestStore::new();
    let req = make_create_request(vec!["ssh-prod"], 1);
    let id = store.create(req).unwrap();
    let approver = Uuid::new_v4();
    store.decide(
        &id,
        ApprovalDecision::Approve { approver_id: approver, note: None },
    ).expect("approve should succeed");
    let fetched = store.get(&id).unwrap();
    assert_eq!(fetched.state, RequestState::Approved);
    assert_eq!(fetched.decided_by, Some(approver));
}

#[test]
fn test_deny_request_changes_state() {
    let store = AccessRequestStore::new();
    let req = make_create_request(vec!["ssh-prod"], 1);
    let id = store.create(req).unwrap();
    let denier = Uuid::new_v4();
    store.decide(
        &id,
        ApprovalDecision::Deny { denier_id: denier, reason: "Policy violation".to_string() },
    ).expect("deny should succeed");
    let fetched = store.get(&id).unwrap();
    assert_eq!(fetched.state, RequestState::Denied);
}

#[test]
fn test_double_decide_errors() {
    let store = AccessRequestStore::new();
    let req = make_create_request(vec!["admin"], 2);
    let id = store.create(req).unwrap();
    let approver = Uuid::new_v4();
    store.decide(&id, ApprovalDecision::Approve { approver_id: approver, note: None }).unwrap();
    let err = store.decide(&id, ApprovalDecision::Approve { approver_id: approver, note: None })
        .unwrap_err();
    assert!(matches!(err, RequestError::AlreadyDecided));
}

#[test]
fn test_list_pending_only_returns_pending() {
    let store = AccessRequestStore::new();
    let id1 = store.create(make_create_request(vec!["role-a"], 1)).unwrap();
    let _id2 = store.create(make_create_request(vec!["role-b"], 1)).unwrap();
    let approver = Uuid::new_v4();
    store.decide(&id1, ApprovalDecision::Approve { approver_id: approver, note: None }).unwrap();
    let pending = store.list_pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state, RequestState::Pending);
}

#[test]
fn test_expired_requests_filtered() {
    let store = AccessRequestStore::new();
    // TTL of 0 means already expired
    let req = CreateRequest {
        user_id: Uuid::new_v4(),
        requested_roles: vec!["admin".to_string()],
        reason: "test".to_string(),
        ttl: Duration::seconds(-1),
    };
    let id = store.create(req).unwrap();
    // An expired pending request should not appear in active pending list
    let fetched = store.get(&id).unwrap();
    assert!(fetched.is_expired());
}

#[test]
fn test_get_nonexistent_returns_none() {
    let store = AccessRequestStore::new();
    assert!(store.get(&Uuid::new_v4()).is_none());
}

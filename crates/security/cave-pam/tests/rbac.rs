// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for PAM RBAC role definitions and policy evaluation.

use cave_pam::rbac::{
    Action, Effect, PolicyEngine, Resource, ResourceKind, Role, RoleAssignment,
    RoleStore,
};
use uuid::Uuid;

fn make_role(name: &str, actions: Vec<(ResourceKind, Action, Effect)>) -> Role {
    Role {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: format!("Role: {name}"),
        rules: actions
            .into_iter()
            .map(|(kind, action, effect)| cave_pam::rbac::PolicyRule {
                resource_kind: kind,
                action,
                effect,
                resource_selector: cave_pam::rbac::ResourceSelector::All,
            })
            .collect(),
    }
}

#[test]
fn test_allow_rule_permits_matching_action() {
    let store = RoleStore::new();
    let role = make_role(
        "db-read",
        vec![(ResourceKind::Database, Action::Connect, Effect::Allow)],
    );
    let role_id = store.register(role).unwrap();
    let user_id = Uuid::new_v4();
    store.assign(RoleAssignment { user_id, role_id }).unwrap();

    let engine = PolicyEngine::new(store);
    let resource = Resource {
        kind: ResourceKind::Database,
        name: "postgres-prod".to_string(),
    };
    assert!(engine.is_allowed(&user_id, &resource, Action::Connect));
}

#[test]
fn test_no_role_denies_action() {
    let store = RoleStore::new();
    let engine = PolicyEngine::new(store);
    let user_id = Uuid::new_v4();
    let resource = Resource {
        kind: ResourceKind::Server,
        name: "bastion-01".to_string(),
    };
    assert!(!engine.is_allowed(&user_id, &resource, Action::Connect));
}

#[test]
fn test_deny_effect_overrides_allow() {
    let store = RoleStore::new();
    // Allow all servers, but deny a specific label via a second role.
    let allow_role = make_role(
        "server-access",
        vec![(ResourceKind::Server, Action::Connect, Effect::Allow)],
    );
    let deny_role = make_role(
        "server-deny",
        vec![(ResourceKind::Server, Action::Connect, Effect::Deny)],
    );
    let allow_id = store.register(allow_role).unwrap();
    let deny_id = store.register(deny_role).unwrap();
    let user_id = Uuid::new_v4();
    store.assign(RoleAssignment { user_id, role_id: allow_id }).unwrap();
    store.assign(RoleAssignment { user_id, role_id: deny_id }).unwrap();

    let engine = PolicyEngine::new(store);
    let resource = Resource {
        kind: ResourceKind::Server,
        name: "bastion-01".to_string(),
    };
    // Deny takes priority.
    assert!(!engine.is_allowed(&user_id, &resource, Action::Connect));
}

#[test]
fn test_wrong_resource_kind_not_permitted() {
    let store = RoleStore::new();
    let role = make_role(
        "db-only",
        vec![(ResourceKind::Database, Action::Connect, Effect::Allow)],
    );
    let role_id = store.register(role).unwrap();
    let user_id = Uuid::new_v4();
    store.assign(RoleAssignment { user_id, role_id }).unwrap();
    let engine = PolicyEngine::new(store);
    // User has DB access but not Server access.
    let resource = Resource { kind: ResourceKind::Server, name: "srv-01".to_string() };
    assert!(!engine.is_allowed(&user_id, &resource, Action::Connect));
}

#[test]
fn test_role_deassign_removes_access() {
    let store = RoleStore::new();
    let role = make_role(
        "admin",
        vec![(ResourceKind::Server, Action::Connect, Effect::Allow)],
    );
    let role_id = store.register(role).unwrap();
    let user_id = Uuid::new_v4();
    store.assign(RoleAssignment { user_id, role_id }).unwrap();
    store.deassign(&user_id, &role_id).unwrap();
    let engine = PolicyEngine::new(store);
    let resource = Resource { kind: ResourceKind::Server, name: "srv-01".to_string() };
    assert!(!engine.is_allowed(&user_id, &resource, Action::Connect));
}

#[test]
fn test_list_roles_for_user() {
    let store = RoleStore::new();
    let role1 = make_role("r1", vec![(ResourceKind::Database, Action::Connect, Effect::Allow)]);
    let role2 = make_role("r2", vec![(ResourceKind::Kubernetes, Action::Exec, Effect::Allow)]);
    let id1 = store.register(role1).unwrap();
    let id2 = store.register(role2).unwrap();
    let user_id = Uuid::new_v4();
    store.assign(RoleAssignment { user_id, role_id: id1 }).unwrap();
    store.assign(RoleAssignment { user_id, role_id: id2 }).unwrap();
    let roles = store.roles_for_user(&user_id);
    assert_eq!(roles.len(), 2);
}

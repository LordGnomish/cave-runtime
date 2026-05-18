// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts cross-crate integration tests (cave-auth)
//! Integration test — Harbor RBAC delegated to cave-auth's identity model.
//!
//! Exercises the real cave-auth `AuthContext` + `BindingScope` /
//! `RoleBinding` shapes through the `HarborAuthBridge`. No mocks: every
//! type is the production cave-auth definition.

use cave_artifacts::integrations::auth::{HarborAuthBridge, HarborRole};
use cave_auth::auth_middleware::AuthContext;
use cave_auth::rbac::{BindingScope, RoleBinding};
use cave_core::types::{CaveRole, TokenType};
use uuid::Uuid;

fn ctx(uid: Uuid, perms: &[&str]) -> AuthContext {
    AuthContext {
        cave_uid: uid,
        email: Some("alice@cave.dev".into()),
        roles: vec![CaveRole::Developer],
        permissions: perms.iter().map(|s| s.to_string()).collect(),
        groups: vec!["dev-team".into()],
        okta_claims: serde_json::json!({"sub": uid.to_string()}),
        token_type: TokenType::Jwt,
    }
}

#[test]
fn end_to_end_oidc_request_to_harbor_action_decision_with_project_binding() {
    let bridge = HarborAuthBridge::new();
    let alice = Uuid::new_v4();
    // Operator bound Alice as Developer on `library` via the portal.
    bridge.bind_user_to_project_role(alice, "acme", "library", HarborRole::Developer);
    // Real cave-auth context (would come from `AuthLayer` JWT decode).
    let c = ctx(alice, &[]);
    // Allowed: pull + push.
    assert!(bridge.may_for_context(&c, "acme", "library", "pull"));
    assert!(bridge.may_for_context(&c, "acme", "library", "push"));
    // Denied: scan, admin (need Maintainer / ProjectAdmin).
    assert!(!bridge.may_for_context(&c, "acme", "library", "scan"));
    assert!(!bridge.may_for_context(&c, "acme", "library", "admin"));
    // Denied: different project.
    assert!(!bridge.may_for_context(&c, "acme", "secret-vault", "push"));
}

#[test]
fn explicit_cave_auth_permission_bypasses_project_lookup() {
    let bridge = HarborAuthBridge::new();
    let bob = Uuid::new_v4();
    // No binding registered.
    // But cave-auth has already granted `cave-artifacts:scan` (e.g. via
    // a security-engineer Role assigned at platform scope inside
    // cave-auth's RbacEngine).
    let c = ctx(bob, &["cave-artifacts:scan"]);
    assert!(bridge.may_for_context(&c, "acme", "library", "scan"));
    // Other actions still denied (no wildcard).
    assert!(!bridge.may_for_context(&c, "acme", "library", "delete"));
}

#[test]
fn wildcard_grant_in_cave_auth_covers_every_action() {
    let bridge = HarborAuthBridge::new();
    let admin = Uuid::new_v4();
    let c = ctx(admin, &["cave-artifacts:*"]);
    for action in ["pull", "push", "delete", "scan", "admin"] {
        assert!(
            bridge.may_for_context(&c, "acme", "library", action),
            "wildcard should permit {action}"
        );
    }
}

#[test]
fn binding_with_full_role_binding_shape_round_trips_via_cave_auth_types() {
    // Verifies we accept the same RoleBinding shape cave-auth itself uses
    // (so a future migration to cave-auth's RbacEngine just copies the
    // Vec<RoleBinding> across).
    let bridge = HarborAuthBridge::new();
    let alice = Uuid::new_v4();
    let binding = RoleBinding {
        binding_id: Uuid::new_v4(),
        cave_uid: alice,
        role: "harbor:maintainer".into(),
        scope: BindingScope::Project {
            tenant_id: "acme".into(),
            project_id: "library".into(),
        },
    };
    bridge.add_binding(binding);
    assert_eq!(
        bridge.resolve_project_role(alice, "acme", "library"),
        Some(HarborRole::Maintainer)
    );
}

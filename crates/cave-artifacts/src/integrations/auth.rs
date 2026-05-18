// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts integrations::auth (Harbor RBAC ↔ cave-auth bridge)
//! Harbor RBAC ↔ `cave-auth` bridge.
//!
//! Maps Harbor's `Project.role_id` enum (Guest/Developer/Maintainer/Admin)
//! into a binding store keyed by cave-auth's `(cave_uid, scope)` shape so
//! that one OIDC token issued by `cave-auth` (against Okta or Keycloak)
//! cleanly drives Harbor project-scoped permissions without Harbor having
//! to talk to the IdP itself.
//!
//! Upstream sources:
//! - goharbor/harbor `src/pkg/permission/types/role.go`         (Guest/Developer/…)
//! - goharbor/harbor `src/pkg/oidc/manager.go`                  (OIDC token verify)
//! - cave-auth `crates/cave-auth/src/rbac.rs`                   (BindingScope, RoleBinding shape)
//! - cave-auth `crates/cave-auth/src/auth_middleware.rs`        (AuthContext extractor)
//!
//! Wiring shape:
//!
//! ```text
//! Incoming request (Bearer JWT)
//!     │
//!     ▼
//! cave-auth AuthLayer (verifies JWT signature, issuer, audience)
//!     │
//!     ▼
//! AuthContext{ cave_uid, groups, tenant_id }
//!     │
//!     ▼
//! HarborAuthBridge::resolve_project_role(uid, project_name)
//!     │   (consults the bridge's binding store; falls back to None when
//!     │    no binding exists and Harbor's per-project ACL should answer)
//!     ▼
//! HarborRole { Guest | Developer | Maintainer | ProjectAdmin }
//! ```
//!
//! The bridge owns its own binding table rather than mutating cave-auth's
//! `RbacEngine.roles` (immutable Arc<HashMap>). It reads cave-auth's
//! [`BindingScope`] + [`RoleBinding`] data types directly so the wire
//! shape stays compatible — a future migration that promotes the bridge
//! into cave-auth proper is a copy of bindings, no schema change.

use cave_auth::auth_middleware::AuthContext;
use cave_auth::rbac::{BindingScope, RoleBinding};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use uuid::Uuid;

/// Harbor's project-level role enum. Source: goharbor/harbor
/// `src/pkg/permission/types/role.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarborRole {
    /// Anonymous public-project pull.
    Guest,
    /// Push + pull.
    Developer,
    /// Developer + scan + delete.
    Maintainer,
    /// Maintainer + member-management + project-deletion.
    ProjectAdmin,
}

impl HarborRole {
    /// Wire name used in Harbor REST + the dashboard label.
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Guest => "guest",
            Self::Developer => "developer",
            Self::Maintainer => "maintainer",
            Self::ProjectAdmin => "projectAdmin",
        }
    }

    /// Suffix used in the `harbor:<suffix>` role-name convention.
    pub fn as_wire_role_suffix(&self) -> &'static str {
        match self {
            Self::Guest => "guest",
            Self::Developer => "developer",
            Self::Maintainer => "maintainer",
            Self::ProjectAdmin => "project-admin",
        }
    }

    /// Maps a cave-auth role name into a Harbor project role. Convention:
    /// cave-auth roles for the Harbor module are `harbor:<role>`.
    pub fn from_cave_role(name: &str) -> Option<Self> {
        match name.strip_prefix("harbor:") {
            Some("guest") => Some(Self::Guest),
            Some("developer") => Some(Self::Developer),
            Some("maintainer") => Some(Self::Maintainer),
            Some("project-admin") | Some("projectAdmin") => Some(Self::ProjectAdmin),
            _ => None,
        }
    }

    /// Translate into the set of allowed actions on the harbor module.
    /// Mirrors Harbor's `permissionPolicies` table.
    pub fn allowed_actions(&self) -> &'static [&'static str] {
        match self {
            Self::Guest => &["pull"],
            Self::Developer => &["pull", "push"],
            Self::Maintainer => &["pull", "push", "delete", "scan"],
            Self::ProjectAdmin => &["pull", "push", "delete", "scan", "admin"],
        }
    }
}

/// Bridge owning its own binding store + scope-precedence lookup. Reads
/// cave-auth's [`BindingScope`] / [`RoleBinding`] shapes directly so the
/// stored data is wire-compatible with the rest of cave-auth.
#[derive(Default)]
pub struct HarborAuthBridge {
    bindings: RwLock<Vec<RoleBinding>>,
}

impl HarborAuthBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a binding. Convention: `binding.role` must be one of
    /// `harbor:<guest|developer|maintainer|project-admin>`.
    pub fn add_binding(&self, b: RoleBinding) {
        self.bindings.write().unwrap().push(b);
    }

    /// Convenience helper for the portal "add member" form.
    pub fn bind_user_to_project_role(
        &self,
        uid: Uuid,
        tenant_id: &str,
        project: &str,
        role: HarborRole,
    ) {
        self.add_binding(RoleBinding {
            binding_id: Uuid::new_v4(),
            cave_uid: uid,
            role: format!("harbor:{}", role.as_wire_role_suffix()),
            scope: BindingScope::Project {
                tenant_id: tenant_id.to_string(),
                project_id: project.to_string(),
            },
        });
    }

    /// Effective role for `uid` on `project`. Scope precedence:
    /// **Project > Tenant > Platform**. Returns `None` when no binding
    /// applies — caller may fall through to Harbor's per-project ACL.
    pub fn resolve_project_role(&self, uid: Uuid, tenant_id: &str, project: &str) -> Option<HarborRole> {
        let bindings = self.bindings.read().unwrap();
        let user_bindings: Vec<&RoleBinding> =
            bindings.iter().filter(|b| b.cave_uid == uid).collect();
        // 1) project-scope exact match
        for b in &user_bindings {
            if let BindingScope::Project { tenant_id: t, project_id: p } = &b.scope {
                if t == tenant_id && p == project {
                    if let Some(r) = HarborRole::from_cave_role(&b.role) {
                        return Some(r);
                    }
                }
            }
        }
        // 2) tenant-scope match
        for b in &user_bindings {
            if let BindingScope::Tenant { tenant_id: t } = &b.scope {
                if t == tenant_id {
                    if let Some(r) = HarborRole::from_cave_role(&b.role) {
                        return Some(r);
                    }
                }
            }
        }
        // 3) platform-scope match
        for b in &user_bindings {
            if matches!(b.scope, BindingScope::Platform) {
                if let Some(r) = HarborRole::from_cave_role(&b.role) {
                    return Some(r);
                }
            }
        }
        None
    }

    /// True when `uid` may perform `action` on `project`.
    pub fn may(&self, uid: Uuid, tenant_id: &str, project: &str, action: &str) -> bool {
        match self.resolve_project_role(uid, tenant_id, project) {
            Some(role) => role.allowed_actions().iter().any(|a| *a == action),
            None => false,
        }
    }

    /// Resolve against a cave-auth [`AuthContext`] — the type the real
    /// `AuthLayer` injects into every authenticated request. This is the
    /// shape Harbor REST handlers actually receive.
    pub fn may_for_context(
        &self,
        ctx: &AuthContext,
        tenant_id: &str,
        project: &str,
        action: &str,
    ) -> bool {
        // Honour cave-auth's explicit permission grant first (covers
        // platform-admin override; cave-auth seeds platform-admin with `*`).
        let direct = format!("cave-artifacts:{action}");
        if ctx.has_permission(&direct) || ctx.has_permission("cave-artifacts:*") {
            return true;
        }
        self.may(ctx.cave_uid, tenant_id, project, action)
    }

    /// Number of bindings stored — used by the portal RBAC tab.
    pub fn binding_count(&self) -> usize {
        self.bindings.read().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_core::types::{CaveRole, TokenType};

    fn ctx(uid: Uuid, perms: &[&str]) -> AuthContext {
        AuthContext {
            cave_uid: uid,
            email: Some("test@cave.dev".into()),
            roles: vec![CaveRole::Developer],
            permissions: perms.iter().map(|s| s.to_string()).collect(),
            groups: vec![],
            okta_claims: serde_json::json!({}),
            token_type: TokenType::Jwt,
        }
    }

    #[test]
    fn role_wire_round_trip() {
        for role in [
            HarborRole::Guest,
            HarborRole::Developer,
            HarborRole::Maintainer,
            HarborRole::ProjectAdmin,
        ] {
            let cave_name = format!("harbor:{}", role.as_wire_role_suffix());
            let back = HarborRole::from_cave_role(&cave_name).unwrap();
            assert_eq!(role, back);
        }
    }

    #[test]
    fn role_accepts_legacy_camelcase_admin_suffix() {
        assert_eq!(
            HarborRole::from_cave_role("harbor:projectAdmin"),
            Some(HarborRole::ProjectAdmin)
        );
    }

    #[test]
    fn role_action_matrix_matches_harbor() {
        assert_eq!(HarborRole::Guest.allowed_actions(), &["pull"]);
        assert_eq!(HarborRole::Developer.allowed_actions(), &["pull", "push"]);
        assert_eq!(
            HarborRole::Maintainer.allowed_actions(),
            &["pull", "push", "delete", "scan"]
        );
        assert_eq!(
            HarborRole::ProjectAdmin.allowed_actions(),
            &["pull", "push", "delete", "scan", "admin"]
        );
    }

    #[test]
    fn bridge_resolves_project_role_from_binding() {
        let bridge = HarborAuthBridge::new();
        let uid = Uuid::new_v4();
        bridge.bind_user_to_project_role(uid, "acme", "library", HarborRole::Developer);
        assert_eq!(
            bridge.resolve_project_role(uid, "acme", "library"),
            Some(HarborRole::Developer)
        );
        assert!(bridge.may(uid, "acme", "library", "push"));
        assert!(bridge.may(uid, "acme", "library", "pull"));
        assert!(!bridge.may(uid, "acme", "library", "admin"));
    }

    #[test]
    fn bridge_returns_none_when_no_binding_exists() {
        let bridge = HarborAuthBridge::new();
        let uid = Uuid::new_v4();
        assert!(bridge.resolve_project_role(uid, "acme", "library").is_none());
        assert!(!bridge.may(uid, "acme", "library", "pull"));
    }

    #[test]
    fn bridge_tenant_scope_falls_through_when_no_project_scope() {
        let bridge = HarborAuthBridge::new();
        let uid = Uuid::new_v4();
        bridge.add_binding(RoleBinding {
            binding_id: Uuid::new_v4(),
            cave_uid: uid,
            role: "harbor:maintainer".into(),
            scope: BindingScope::Tenant {
                tenant_id: "acme".into(),
            },
        });
        assert_eq!(
            bridge.resolve_project_role(uid, "acme", "anything"),
            Some(HarborRole::Maintainer)
        );
    }

    #[test]
    fn bridge_project_scope_wins_over_tenant_scope() {
        let bridge = HarborAuthBridge::new();
        let uid = Uuid::new_v4();
        bridge.add_binding(RoleBinding {
            binding_id: Uuid::new_v4(),
            cave_uid: uid,
            role: "harbor:guest".into(),
            scope: BindingScope::Tenant {
                tenant_id: "acme".into(),
            },
        });
        bridge.bind_user_to_project_role(uid, "acme", "library", HarborRole::ProjectAdmin);
        assert_eq!(
            bridge.resolve_project_role(uid, "acme", "library"),
            Some(HarborRole::ProjectAdmin)
        );
    }

    #[test]
    fn bridge_platform_scope_is_last_resort() {
        let bridge = HarborAuthBridge::new();
        let uid = Uuid::new_v4();
        bridge.add_binding(RoleBinding {
            binding_id: Uuid::new_v4(),
            cave_uid: uid,
            role: "harbor:project-admin".into(),
            scope: BindingScope::Platform,
        });
        assert_eq!(
            bridge.resolve_project_role(uid, "anywhere", "anything"),
            Some(HarborRole::ProjectAdmin)
        );
    }

    #[test]
    fn may_for_context_honours_explicit_permission_first() {
        let bridge = HarborAuthBridge::new();
        let uid = Uuid::new_v4();
        // No binding — but the context already carries the permission.
        let c = ctx(uid, &["cave-artifacts:push"]);
        assert!(bridge.may_for_context(&c, "acme", "library", "push"));
        // Wildcard works too.
        let c2 = ctx(uid, &["cave-artifacts:*"]);
        assert!(bridge.may_for_context(&c2, "acme", "library", "scan"));
        // Without permission + without binding → deny.
        let c3 = ctx(uid, &[]);
        assert!(!bridge.may_for_context(&c3, "acme", "library", "push"));
    }

    #[test]
    fn binding_count_reflects_inserts() {
        let bridge = HarborAuthBridge::new();
        assert_eq!(bridge.binding_count(), 0);
        bridge.bind_user_to_project_role(Uuid::new_v4(), "t", "p", HarborRole::Guest);
        bridge.bind_user_to_project_role(Uuid::new_v4(), "t", "p", HarborRole::Developer);
        assert_eq!(bridge.binding_count(), 2);
    }
}

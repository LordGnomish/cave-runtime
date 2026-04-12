//! Role-Based Access Control (RBAC).
//!
//! Built-in roles: admin, editor, viewer (plus tenant-scoped variants).
//! Supports custom roles with granular permissions and role assignments.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// A permission in the form "module:action", e.g., "secrets:write".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Permission(pub String);

impl Permission {
    pub fn new(module: &str, action: &str) -> Self {
        Self(format!("{module}:{action}"))
    }

    pub fn module(&self) -> &str {
        self.0.split(':').next().unwrap_or("")
    }

    pub fn action(&self) -> &str {
        self.0.split(':').nth(1).unwrap_or("")
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A role definition with associated permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// None = platform-wide role; Some(t) = scoped to tenant t.
    pub tenant_id: Option<String>,
    pub permissions: HashSet<Permission>,
    pub built_in: bool,
    pub created_at: DateTime<Utc>,
}

impl Role {
    pub fn new(name: &str, description: &str, permissions: Vec<Permission>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: description.to_string(),
            tenant_id: None,
            permissions: permissions.into_iter().collect(),
            built_in: false,
            created_at: Utc::now(),
        }
    }

    pub fn has_permission(&self, perm: &Permission) -> bool {
        // Wildcard: "module:*" grants all actions on that module
        let wildcard = Permission::new(perm.module(), "*");
        self.permissions.contains(perm) || self.permissions.contains(&wildcard)
    }
}

/// Built-in roles with their default permissions.
pub fn built_in_roles() -> Vec<Role> {
    let admin = Role {
        id: Uuid::new_v4(),
        name: "admin".to_string(),
        description: "Full platform access".to_string(),
        tenant_id: None,
        permissions: {
            let mut p = HashSet::new();
            p.insert(Permission::new("*", "*")); // All permissions
            p
        },
        built_in: true,
        created_at: Utc::now(),
    };

    let editor = Role {
        id: Uuid::new_v4(),
        name: "editor".to_string(),
        description: "Read and write access, no admin operations".to_string(),
        tenant_id: None,
        permissions: [
            ("flags", vec!["read", "write"]),
            ("secrets", vec!["read", "write"]),
            ("docs", vec!["read", "write"]),
            ("vulns", vec!["read", "write", "triage"]),
            ("sbom", vec!["read", "write"]),
            ("scan", vec!["read", "write", "trigger"]),
            ("registry", vec!["read", "write"]),
            ("portal", vec!["read", "write"]),
            ("workflows", vec!["read", "write", "execute"]),
            ("incidents", vec!["read", "write"]),
            ("chat", vec!["read", "write"]),
            ("cost", vec!["read"]),
            ("devlake", vec!["read"]),
        ]
        .iter()
        .flat_map(|(module, actions)| {
            actions.iter().map(|action| Permission::new(module, action))
        })
        .collect(),
        built_in: true,
        created_at: Utc::now(),
    };

    let viewer = Role {
        id: Uuid::new_v4(),
        name: "viewer".to_string(),
        description: "Read-only access to all resources".to_string(),
        tenant_id: None,
        permissions: [
            "flags", "secrets", "docs", "vulns", "sbom", "scan", "registry",
            "portal", "workflows", "incidents", "chat", "cost", "devlake",
            "uptime", "slo", "status",
        ]
        .iter()
        .map(|module| Permission::new(module, "read"))
        .collect(),
        built_in: true,
        created_at: Utc::now(),
    };

    vec![admin, editor, viewer]
}

/// A role assignment — links a user to a role within a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub id: Uuid,
    pub user_id: Uuid,
    pub role_id: Uuid,
    pub tenant_id: String,
    pub assigned_by: Uuid,
    pub assigned_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl RoleAssignment {
    pub fn new(user_id: Uuid, role_id: Uuid, tenant_id: String, assigned_by: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            role_id,
            tenant_id,
            assigned_by,
            assigned_at: Utc::now(),
            expires_at: None,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Utc::now() > exp)
            .unwrap_or(false)
    }
}

/// RBAC engine — manages roles, permissions, and assignments.
#[derive(Clone)]
pub struct RbacEngine {
    roles: Arc<RwLock<HashMap<Uuid, Role>>>,
    assignments: Arc<RwLock<Vec<RoleAssignment>>>,
}

impl RbacEngine {
    pub fn new() -> Self {
        let mut roles = HashMap::new();
        for role in built_in_roles() {
            roles.insert(role.id, role);
        }
        Self {
            roles: Arc::new(RwLock::new(roles)),
            assignments: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create a custom role.
    pub async fn create_role(&self, role: Role) -> Uuid {
        let id = role.id;
        self.roles.write().await.insert(id, role);
        id
    }

    /// Look up a role by name.
    pub async fn role_by_name(&self, name: &str) -> Option<Role> {
        self.roles
            .read()
            .await
            .values()
            .find(|r| r.name == name)
            .cloned()
    }

    /// Assign a role to a user.
    pub async fn assign_role(&self, assignment: RoleAssignment) {
        self.assignments.write().await.push(assignment);
    }

    /// Revoke all role assignments for a user within a tenant.
    pub async fn revoke_role(&self, user_id: Uuid, role_id: Uuid, tenant_id: &str) {
        let mut assignments = self.assignments.write().await;
        assignments.retain(|a| {
            !(a.user_id == user_id && a.role_id == role_id && a.tenant_id == tenant_id)
        });
    }

    /// Get all active roles for a user in a tenant.
    pub async fn user_roles(&self, user_id: Uuid, tenant_id: &str) -> Vec<Role> {
        let assignments = self.assignments.read().await;
        let roles = self.roles.read().await;
        assignments
            .iter()
            .filter(|a| a.user_id == user_id && a.tenant_id == tenant_id && !a.is_expired())
            .filter_map(|a| roles.get(&a.role_id).cloned())
            .collect()
    }

    /// Check if a user has a specific permission in a tenant.
    pub async fn check_permission(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        permission: &Permission,
    ) -> bool {
        let roles = self.user_roles(user_id, tenant_id).await;
        for role in &roles {
            // Global wildcard
            if role.permissions.contains(&Permission::new("*", "*")) {
                return true;
            }
            if role.has_permission(permission) {
                return true;
            }
        }
        false
    }
}

impl Default for RbacEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_module_action_parse() {
        let p = Permission::new("secrets", "write");
        assert_eq!(p.module(), "secrets");
        assert_eq!(p.action(), "write");
        assert_eq!(p.to_string(), "secrets:write");
    }

    #[test]
    fn built_in_roles_exist() {
        let roles = built_in_roles();
        let names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"admin"));
        assert!(names.contains(&"editor"));
        assert!(names.contains(&"viewer"));
    }

    #[test]
    fn admin_role_has_wildcard() {
        let roles = built_in_roles();
        let admin = roles.iter().find(|r| r.name == "admin").unwrap();
        assert!(admin.permissions.contains(&Permission::new("*", "*")));
    }

    #[test]
    fn role_wildcard_permission_check() {
        let mut role = Role::new("superadmin", "desc", vec![]);
        role.permissions.insert(Permission::new("secrets", "*"));
        assert!(role.has_permission(&Permission::new("secrets", "read")));
        assert!(role.has_permission(&Permission::new("secrets", "delete")));
        assert!(!role.has_permission(&Permission::new("flags", "read")));
    }

    #[tokio::test]
    async fn rbac_assign_and_check_permission() {
        let engine = RbacEngine::new();
        let admin_id = engine.role_by_name("admin").await.unwrap().id;
        let user_id = Uuid::new_v4();
        let assigner = Uuid::new_v4();

        engine
            .assign_role(RoleAssignment::new(user_id, admin_id, "acme".to_string(), assigner))
            .await;

        assert!(
            engine
                .check_permission(user_id, "acme", &Permission::new("secrets", "delete"))
                .await
        );
    }

    #[tokio::test]
    async fn rbac_viewer_cannot_write() {
        let engine = RbacEngine::new();
        let viewer_id = engine.role_by_name("viewer").await.unwrap().id;
        let user_id = Uuid::new_v4();

        engine
            .assign_role(RoleAssignment::new(
                user_id,
                viewer_id,
                "acme".to_string(),
                Uuid::new_v4(),
            ))
            .await;

        assert!(
            !engine
                .check_permission(user_id, "acme", &Permission::new("secrets", "write"))
                .await
        );
        assert!(
            engine
                .check_permission(user_id, "acme", &Permission::new("secrets", "read"))
                .await
        );
    }

    #[tokio::test]
    async fn rbac_revoke_removes_permission() {
        let engine = RbacEngine::new();
        let editor_id = engine.role_by_name("editor").await.unwrap().id;
        let user_id = Uuid::new_v4();

        engine
            .assign_role(RoleAssignment::new(
                user_id,
                editor_id,
                "acme".to_string(),
                Uuid::new_v4(),
            ))
            .await;

        // Has write before revoke
        assert!(
            engine
                .check_permission(user_id, "acme", &Permission::new("flags", "write"))
                .await
        );

        engine.revoke_role(user_id, editor_id, "acme").await;

        // No longer has write
        assert!(
            !engine
                .check_permission(user_id, "acme", &Permission::new("flags", "write"))
                .await
        );
    }

    #[tokio::test]
    async fn rbac_tenant_isolation() {
        let engine = RbacEngine::new();
        let admin_id = engine.role_by_name("admin").await.unwrap().id;
        let user_id = Uuid::new_v4();

        engine
            .assign_role(RoleAssignment::new(
                user_id,
                admin_id,
                "tenant-a".to_string(),
                Uuid::new_v4(),
            ))
            .await;

        // Has permission in tenant-a
        assert!(
            engine
                .check_permission(user_id, "tenant-a", &Permission::new("secrets", "write"))
                .await
        );

        // Does NOT have permission in tenant-b
        assert!(
            !engine
                .check_permission(user_id, "tenant-b", &Permission::new("secrets", "write"))
                .await
        );
    }

    #[tokio::test]
    async fn rbac_create_custom_role() {
        let engine = RbacEngine::new();
        let custom = Role::new(
            "billing-viewer",
            "Read-only billing access",
            vec![Permission::new("cost", "read"), Permission::new("cost", "export")],
        );
        let id = engine.create_role(custom).await;
        let user_id = Uuid::new_v4();

        engine
            .assign_role(RoleAssignment::new(
                user_id,
                id,
                "acme".to_string(),
                Uuid::new_v4(),
            ))
            .await;

        assert!(
            engine
                .check_permission(user_id, "acme", &Permission::new("cost", "read"))
                .await
        );
        assert!(
            !engine
                .check_permission(user_id, "acme", &Permission::new("secrets", "read"))
                .await
        );
    }
}

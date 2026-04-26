//! Pulp v3 RBAC — roles, permissions per object, domain-aware.

use crate::models::ContentType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Permissions ─────────────────────────────────────────────────────────────

/// Pulp permission codename pattern: "app.action_model"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Permission(pub String);

impl Permission {
    pub fn new(app: &str, action: &str, model: &str) -> Self {
        Self(format!("{}.{}_{}", app, action, model))
    }

    pub fn matches(&self, other: &str) -> bool {
        // Wildcard: "core.*" matches any "core.*"
        if self.0.ends_with(".*") {
            let prefix = &self.0[..self.0.len() - 2];
            return other.starts_with(prefix);
        }
        self.0 == other
    }
}

/// Built-in Pulp permissions.
pub fn repository_permissions() -> Vec<Permission> {
    vec![
        Permission::new("core", "view", "repository"),
        Permission::new("core", "add", "repository"),
        Permission::new("core", "change", "repository"),
        Permission::new("core", "delete", "repository"),
        Permission::new("core", "manage_roles", "repository"),
        Permission::new("core", "modify", "repository"),
        Permission::new("core", "sync", "repository"),
        Permission::new("core", "repair", "repository"),
    ]
}

pub fn artifact_permissions() -> Vec<Permission> {
    vec![
        Permission::new("core", "view", "artifact"),
        Permission::new("core", "add", "artifact"),
        Permission::new("core", "delete", "artifact"),
    ]
}

pub fn distribution_permissions() -> Vec<Permission> {
    vec![
        Permission::new("core", "view", "distribution"),
        Permission::new("core", "add", "distribution"),
        Permission::new("core", "change", "distribution"),
        Permission::new("core", "delete", "distribution"),
        Permission::new("core", "manage_roles", "distribution"),
    ]
}

// ─── Built-in role definitions ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinRole {
    pub name: &'static str,
    pub description: &'static str,
    pub permissions: Vec<&'static str>,
    pub locked: bool,
}

pub fn builtin_roles() -> Vec<BuiltinRole> {
    vec![
        BuiltinRole {
            name: "core.superuser",
            description: "Can do everything.",
            permissions: vec!["*.*"],
            locked: true,
        },
        BuiltinRole {
            name: "core.viewer",
            description: "Read-only access to all objects.",
            permissions: vec![
                "core.view_repository",
                "core.view_distribution",
                "core.view_publication",
                "core.view_artifact",
                "core.view_task",
            ],
            locked: true,
        },
        BuiltinRole {
            name: "core.task_owner",
            description: "Can manage tasks.",
            permissions: vec!["core.view_task", "core.delete_task", "core.cancel_task"],
            locked: true,
        },
        BuiltinRole {
            name: "core.repository_creator",
            description: "Can create repositories.",
            permissions: vec!["core.add_repository"],
            locked: true,
        },
        BuiltinRole {
            name: "core.repository_owner",
            description: "Full access to a repository.",
            permissions: vec![
                "core.view_repository",
                "core.change_repository",
                "core.delete_repository",
                "core.manage_roles_repository",
                "core.modify_repository",
                "core.sync_repository",
                "core.repair_repository",
            ],
            locked: true,
        },
        BuiltinRole {
            name: "core.artifact_creator",
            description: "Can upload artifacts.",
            permissions: vec!["core.add_artifact", "core.view_artifact"],
            locked: true,
        },
    ]
}

// ─── Object-level RBAC ───────────────────────────────────────────────────────

/// Assignment of a role to a user/group on a specific object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleAssignment {
    pub role: String,
    pub users: Vec<String>,
    pub groups: Vec<String>,
    pub content_object: Option<String>,
}

/// Check if a user has a specific permission on an object.
pub fn user_has_permission(
    user: &str,
    user_groups: &[String],
    permission: &str,
    assignments: &[RoleAssignment],
    object_href: Option<&str>,
    roles: &[BuiltinRole],
) -> bool {
    for assignment in assignments {
        // Check object scope
        let object_matches = match (object_href, &assignment.content_object) {
            (Some(href), Some(obj)) => href == obj,
            (None, None) => true,
            (_, None) => true, // Global assignment applies everywhere
            _ => false,
        };
        if !object_matches { continue; }

        // Check subject membership
        let is_member = assignment.users.iter().any(|u| u == user)
            || user_groups.iter().any(|g| assignment.groups.contains(g));
        if !is_member { continue; }

        // Check permissions via role
        if let Some(role) = roles.iter().find(|r| r.name == assignment.role) {
            for perm in &role.permissions {
                if *perm == "*.*" || *perm == permission || permission.starts_with(&perm.replace("*", "")) {
                    return true;
                }
            }
        }
    }
    false
}

/// Get all permissions for a user across all their role assignments.
pub fn get_user_permissions(
    user: &str,
    user_groups: &[String],
    assignments: &[RoleAssignment],
    roles: &[BuiltinRole],
) -> Vec<String> {
    let mut perms = std::collections::HashSet::new();
    for assignment in assignments {
        let is_member = assignment.users.iter().any(|u| u == user)
            || user_groups.iter().any(|g| assignment.groups.contains(g));
        if !is_member { continue; }

        if let Some(role) = roles.iter().find(|r| r.name == assignment.role) {
            for perm in &role.permissions {
                perms.insert(perm.to_string());
            }
        }
    }
    let mut result: Vec<String> = perms.into_iter().collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_assignment(role: &str, users: &[&str], groups: &[&str], object: Option<&str>) -> RoleAssignment {
        RoleAssignment {
            role: role.to_string(),
            users: users.iter().map(|s| s.to_string()).collect(),
            groups: groups.iter().map(|s| s.to_string()).collect(),
            content_object: object.map(|s| s.to_string()),
        }
    }

    #[test]
    fn permission_exact_match() {
        let p = Permission::new("core", "view", "repository");
        assert!(p.matches("core.view_repository"));
        assert!(!p.matches("core.delete_repository"));
    }

    #[test]
    fn user_has_permission_via_role() {
        let roles = builtin_roles();
        let assignments = vec![make_assignment("core.viewer", &["alice"], &[], None)];
        assert!(user_has_permission(
            "alice", &[], "core.view_repository", &assignments, None, &roles
        ));
        assert!(!user_has_permission(
            "alice", &[], "core.delete_repository", &assignments, None, &roles
        ));
    }

    #[test]
    fn user_has_permission_via_group() {
        let roles = builtin_roles();
        let assignments = vec![make_assignment("core.repository_owner", &[], &["ops-team"], None)];
        let user_groups = vec!["ops-team".to_string()];
        assert!(user_has_permission(
            "bob", &user_groups, "core.sync_repository", &assignments, None, &roles
        ));
    }

    #[test]
    fn user_has_permission_object_scoped() {
        let roles = builtin_roles();
        let repo_href = "/pulp/api/v3/repositories/abc/";
        let assignments = vec![make_assignment(
            "core.repository_owner",
            &["alice"],
            &[],
            Some(repo_href),
        )];
        // Permission on the right object
        assert!(user_has_permission(
            "alice", &[], "core.sync_repository", &assignments, Some(repo_href), &roles
        ));
        // Different object — no access
        assert!(!user_has_permission(
            "alice", &[], "core.sync_repository", &assignments, Some("/pulp/api/v3/repositories/xyz/"), &roles
        ));
    }

    #[test]
    fn superuser_has_all_permissions() {
        let roles = builtin_roles();
        let assignments = vec![make_assignment("core.superuser", &["admin"], &[], None)];
        assert!(user_has_permission(
            "admin", &[], "core.delete_everything", &assignments, None, &roles
        ));
    }

    #[test]
    fn get_user_permissions_aggregates() {
        let roles = builtin_roles();
        let assignments = vec![
            make_assignment("core.viewer", &["alice"], &[], None),
            make_assignment("core.artifact_creator", &["alice"], &[], None),
        ];
        let perms = get_user_permissions("alice", &[], &assignments, &roles);
        assert!(perms.iter().any(|p| p.contains("view_repository")));
        assert!(perms.iter().any(|p| p.contains("add_artifact")));
    }

    #[test]
    fn builtin_roles_locked() {
        let roles = builtin_roles();
        assert!(roles.iter().all(|r| r.locked));
    }

    #[test]
    fn builtin_roles_includes_superuser() {
        let roles = builtin_roles();
        assert!(roles.iter().any(|r| r.name == "core.superuser"));
    }
}

<<<<<<< HEAD
//! Fine-grained Role-Based Access Control (RBAC).
//!
//! ## Model
//!
//! ```text
//! Role  ──has──►  ModulePermissions  (e.g. "cave-flags" → ["read","write"])
//!  │
//!  └──► parent Role  (hierarchy: platform-admin > module-admin > developer > viewer)
//!
//! RoleBinding  ──binds──►  (cave_uid | group) → Role  at a Scope
//! ResourcePolicy  ──grants──►  (cave_uid) → permissions  on a specific resource
//! ```
//!
//! The `RbacEngine` evaluates permissions by:
//! 1. Walking role hierarchy for coarse checks.
//! 2. Checking `RoleBinding`s for the user + requested scope.
//! 3. Consulting `ResourcePolicy` for per-resource ACL overrides.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

// ─── Models ───────────────────────────────────────────────────────────────────

/// A set of actions allowed for a single module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulePermission {
    /// Module slug, e.g. "cave-flags"
    pub module: String,
    /// Allowed actions, e.g. ["read", "write", "manage"]
    pub actions: Vec<String>,
}

impl ModulePermission {
    fn allows(&self, module: &str, action: &str) -> bool {
        self.module == module
            && (self.actions.iter().any(|a| a == action || a == "*"))
    }
}

/// A named role with a permission set and optional parent (for inheritance).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    pub description: String,
    /// Permissions granted directly by this role
    pub permissions: Vec<ModulePermission>,
    /// Name of the parent role (inherit its permissions too)
    pub parent: Option<String>,
}

/// Scope at which a role binding applies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BindingScope {
    /// Applies across the entire platform
    Platform,
    /// Applies within a single tenant
    Tenant { tenant_id: String },
    /// Applies to a specific project within a tenant
    Project {
        tenant_id: String,
        project_id: String,
    },
}

/// Binds a user (or group) to a role at a specific scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBinding {
    pub binding_id: Uuid,
    /// The CAVE user this binding applies to
    pub cave_uid: Uuid,
    /// Name of the role granted
    pub role: String,
    pub scope: BindingScope,
}

/// Per-resource ACL entry granting a user specific permissions on one resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBinding {
    pub cave_uid: Uuid,
    /// E.g. ["cave-flags:write", "cave-flags:read"]
    pub permissions: Vec<String>,
}

/// Policy pinning ACLs to a specific resource instance.
///
/// Example: user X can manage flags in project Y but only view in project Z.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePolicy {
    /// E.g. "project", "flag", "secret"
    pub resource_type: String,
    /// Opaque resource identifier
    pub resource_id: String,
    pub bindings: Vec<ResourceBinding>,
}

// ─── Predefined roles ─────────────────────────────────────────────────────────

/// All CAVE module slugs (mirrored from claims.rs for RBAC population).
const MODULES: &[&str] = &[
    "cave-flags", "cave-secrets", "cave-lint", "cave-docs", "cave-status",
    "cave-changelog", "cave-certs", "cave-vulns", "cave-sbom", "cave-uptime",
    "cave-cost", "cave-sign", "cave-forensics", "cave-devlake", "cave-ai-obs",
    "cave-pii", "cave-incidents", "cave-chat", "cave-slo", "cave-alerts",
    "cave-profiler", "cave-registry", "cave-workflows", "cave-scan",
    "cave-portal", "cave-scaffold", "cave-chaos", "cave-policy", "cave-dast",
    "cave-backup", "cave-pam", "cave-logs", "cave-auth",
];

fn all_module_perms(actions: &[&str]) -> Vec<ModulePermission> {
    MODULES
        .iter()
        .map(|m| ModulePermission {
            module: m.to_string(),
            actions: actions.iter().map(|a| a.to_string()).collect(),
        })
        .collect()
}

/// Build the canonical set of predefined roles.
pub fn predefined_roles() -> Vec<Role> {
    vec![
        Role {
            name: "platform-admin".to_string(),
            description: "Full access to all CAVE modules and platform settings".to_string(),
            permissions: all_module_perms(&["*"]),
            parent: None,
        },
        Role {
            name: "module-admin".to_string(),
            description: "Admin access to a specific module (set via RoleBinding scope)".to_string(),
            permissions: all_module_perms(&["read", "write", "manage", "admin"]),
            parent: Some("developer".to_string()),
        },
        Role {
            name: "developer".to_string(),
            description: "Read + write access across all modules, no destructive admin ops".to_string(),
            permissions: all_module_perms(&["read", "write"]),
            parent: Some("viewer".to_string()),
        },
        Role {
            name: "viewer".to_string(),
            description: "Read-only access to all modules".to_string(),
            permissions: all_module_perms(&["read", "list"]),
            parent: None,
        },
        Role {
            name: "auditor".to_string(),
            description: "Read-only access to audit logs and security events".to_string(),
            permissions: vec![
                ModulePermission { module: "cave-logs".to_string(), actions: vec!["read".to_string(), "list".to_string()] },
                ModulePermission { module: "cave-auth".to_string(), actions: vec!["audit".to_string(), "read".to_string()] },
                ModulePermission { module: "cave-vulns".to_string(), actions: vec!["read".to_string(), "list".to_string()] },
                ModulePermission { module: "cave-incidents".to_string(), actions: vec!["read".to_string(), "list".to_string()] },
                ModulePermission { module: "cave-pii".to_string(), actions: vec!["read".to_string()] },
            ],
            parent: None,
        },
    ]
}

// ─── RBAC Engine ──────────────────────────────────────────────────────────────

/// Evaluates RBAC permissions for a user request.
#[derive(Clone)]
pub struct RbacEngine {
    /// Role definitions keyed by name
    roles: Arc<HashMap<String, Role>>,
    /// Dynamic role bindings (managed at runtime)
    bindings: Arc<RwLock<Vec<RoleBinding>>>,
    /// Per-resource ACL overrides
    resource_policies: Arc<RwLock<Vec<ResourcePolicy>>>,
}

impl RbacEngine {
    /// Create a new engine pre-loaded with the predefined roles.
    pub fn new() -> Self {
        let roles: HashMap<String, Role> = predefined_roles()
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect();

        Self {
            roles: Arc::new(roles),
            bindings: Arc::new(RwLock::new(Vec::new())),
            resource_policies: Arc::new(RwLock::new(Vec::new())),
        }
    }

    // ── Binding management ────────────────────────────────────────────────

    pub async fn add_binding(&self, binding: RoleBinding) {
        self.bindings.write().await.push(binding);
    }

    pub async fn remove_binding(&self, binding_id: Uuid) {
        self.bindings
            .write()
            .await
            .retain(|b| b.binding_id != binding_id);
    }

    pub async fn add_resource_policy(&self, policy: ResourcePolicy) {
        self.resource_policies.write().await.push(policy);
    }

    // ── Permission evaluation ─────────────────────────────────────────────

    /// Check if `cave_uid` is allowed to perform `action` on `module` at `scope`.
    pub async fn is_allowed(
        &self,
        cave_uid: Uuid,
        module: &str,
        action: &str,
        scope: &BindingScope,
    ) -> bool {
        // 1. Check resource-level ACL (most specific, checked first)
        // ResourcePolicy is keyed by resource_id — callers wanting resource-level
        // checks should use `is_allowed_on_resource` instead.

        // 2. Find role bindings that apply to this user + scope
        let bindings = self.bindings.read().await;
        for binding in bindings.iter() {
            if binding.cave_uid != cave_uid {
                continue;
            }
            if !scope_covers(&binding.scope, scope) {
                continue;
            }
            if self.role_allows(&binding.role, module, action) {
=======
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
>>>>>>> claude/great-sanderson
                return true;
            }
        }
        false
    }
<<<<<<< HEAD

    /// Check permission on a specific resource instance (uses ResourcePolicy ACL).
    pub async fn is_allowed_on_resource(
        &self,
        cave_uid: Uuid,
        resource_type: &str,
        resource_id: &str,
        permission: &str, // "module:action" format
    ) -> bool {
        let policies = self.resource_policies.read().await;
        for policy in policies.iter() {
            if policy.resource_type != resource_type || policy.resource_id != resource_id {
                continue;
            }
            for binding in &policy.bindings {
                if binding.cave_uid == cave_uid {
                    if binding.permissions.iter().any(|p| p == permission || p == "*") {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Get all effective permissions for a user across all their role bindings
    /// at a given scope.
    pub async fn effective_permissions(
        &self,
        cave_uid: Uuid,
        scope: &BindingScope,
    ) -> Vec<String> {
        let mut perms = Vec::new();
        let bindings = self.bindings.read().await;

        for binding in bindings.iter() {
            if binding.cave_uid != cave_uid || !scope_covers(&binding.scope, scope) {
                continue;
            }
            let mut role_perms = self.collect_role_permissions(&binding.role);
            perms.append(&mut role_perms);
        }

        perms.sort();
        perms.dedup();
        perms
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn role_allows(&self, role_name: &str, module: &str, action: &str) -> bool {
        let Some(role) = self.roles.get(role_name) else {
            return false;
        };

        // Check direct permissions
        if role.permissions.iter().any(|p| p.allows(module, action)) {
            return true;
        }

        // Recurse into parent role
        if let Some(ref parent) = role.parent {
            return self.role_allows(parent, module, action);
        }

        false
    }

    fn collect_role_permissions(&self, role_name: &str) -> Vec<String> {
        let Some(role) = self.roles.get(role_name) else {
            return vec![];
        };

        let mut perms: Vec<String> = role
            .permissions
            .iter()
            .flat_map(|mp| {
                mp.actions
                    .iter()
                    .map(|a| format!("{}:{}", mp.module, a))
                    .collect::<Vec<_>>()
            })
            .collect();

        if let Some(ref parent) = role.parent {
            perms.append(&mut self.collect_role_permissions(parent));
        }

        perms
    }
=======
>>>>>>> claude/great-sanderson
}

impl Default for RbacEngine {
    fn default() -> Self {
        Self::new()
    }
}

<<<<<<< HEAD
/// Returns true if `binding_scope` covers (is equal to or broader than) `requested`.
fn scope_covers(binding_scope: &BindingScope, requested: &BindingScope) -> bool {
    match binding_scope {
        BindingScope::Platform => true, // platform binding covers everything
        BindingScope::Tenant { tenant_id: bt } => match requested {
            BindingScope::Platform => false,
            BindingScope::Tenant { tenant_id: rt } => bt == rt,
            BindingScope::Project { tenant_id: rt, .. } => bt == rt,
        },
        BindingScope::Project {
            tenant_id: bt,
            project_id: bp,
        } => match requested {
            BindingScope::Project {
                tenant_id: rt,
                project_id: rp,
            } => bt == rt && bp == rp,
            _ => false,
        },
=======
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
>>>>>>> claude/great-sanderson
    }
}

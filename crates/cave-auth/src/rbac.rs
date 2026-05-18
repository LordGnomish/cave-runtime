// SPDX-License-Identifier: AGPL-3.0-or-later
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
                return true;
            }
        }
        false
    }

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
}

impl Default for RbacEngine {
    fn default() -> Self {
        Self::new()
    }
}

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
    }
}

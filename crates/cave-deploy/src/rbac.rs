//! RBAC — project-scoped roles with Casbin-style policies.
//!
//! Policy format mirrors ArgoCD:
//!   `p, <role>, <resource>, <action>, <effect>`
//!
//! Resources: applications, repositories, clusters, projects, logs, exec
//! Actions:   get, create, update, delete, sync, override, action
//! Effects:   allow, deny

use crate::models::AppProject;

/// The subject performing an action (resolved from the JWT identity).
#[derive(Debug, Clone)]
pub struct Subject {
    /// Username or service-account name.
    pub name: String,
    /// SSO group memberships used to resolve project roles.
    pub groups: Vec<String>,
}

/// Check if a subject is allowed to perform `action` on `resource` within
/// the given project.  Returns `true` if any matching role policy grants allow
/// and no policy grants deny.
pub fn check_permission(
    subject: &Subject,
    project: &AppProject,
    resource: &str,
    action: &str,
) -> bool {
    let applicable_roles = resolve_roles(subject, project);

    let mut has_allow = false;

    for role in &applicable_roles {
        for policy in &role.policies {
            match parse_policy(policy) {
                Some(p) if p.matches(resource, action) => {
                    if p.effect == PolicyEffect::Deny {
                        return false; // explicit deny wins immediately
                    }
                    has_allow = true;
                }
                _ => {}
            }
        }
    }

    has_allow
}

/// Resolve which project roles a subject holds (by group membership or name).
fn resolve_roles<'p>(subject: &Subject, project: &'p AppProject) -> Vec<&'p crate::models::ProjectRole> {
    project
        .roles
        .iter()
        .filter(|role| {
            // Direct name match or group membership
            role.name == subject.name
                || role.groups.iter().any(|g| subject.groups.contains(g))
        })
        .collect()
}

// ─── Policy parsing ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum PolicyEffect {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
struct ParsedPolicy {
    /// The role this policy belongs to (unused after resolution, kept for clarity).
    _role: String,
    /// Glob pattern for resource type, e.g. "applications", "*"
    resource: String,
    /// Glob pattern for action, e.g. "sync", "*"
    action: String,
    effect: PolicyEffect,
}

impl ParsedPolicy {
    fn matches(&self, resource: &str, action: &str) -> bool {
        glob_match(&self.resource, resource) && glob_match(&self.action, action)
    }
}

/// Parse "p, role:name, resource, action, allow|deny"
fn parse_policy(policy: &str) -> Option<ParsedPolicy> {
    let parts: Vec<&str> = policy.split(',').map(str::trim).collect();
    if parts.len() != 5 || parts[0] != "p" {
        return None;
    }
    let effect = match parts[4] {
        "allow" => PolicyEffect::Allow,
        "deny" => PolicyEffect::Deny,
        _ => return None,
    };
    Some(ParsedPolicy {
        _role: parts[1].to_string(),
        resource: parts[2].to_string(),
        action: parts[3].to_string(),
        effect,
    })
}

/// Simple glob match supporting "*" as wildcard.
fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    pattern == value
}

// ─── Built-in project roles ───────────────────────────────────────────────────

/// Policies granted to the built-in `role:admin` in every project.
pub const PROJECT_ADMIN_POLICIES: &[&str] = &[
    "p, role:admin, applications, *, allow",
    "p, role:admin, repositories, *, allow",
    "p, role:admin, clusters, get, allow",
    "p, role:admin, exec, *, allow",
    "p, role:admin, logs, get, allow",
];

/// Policies granted to the built-in `role:readonly` in every project.
pub const PROJECT_READONLY_POLICIES: &[&str] = &[
    "p, role:readonly, applications, get, allow",
    "p, role:readonly, repositories, get, allow",
    "p, role:readonly, clusters, get, allow",
];

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AppProject, ApplicationDestination, ProjectRole};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_project(roles: Vec<ProjectRole>) -> AppProject {
        AppProject {
            id: Uuid::new_v4(),
            name: "myproject".to_string(),
            description: None,
            source_repos: vec!["*".to_string()],
            destinations: vec![ApplicationDestination {
                server: Some("https://kubernetes.default.svc".to_string()),
                name: None,
                namespace: "*".to_string(),
            }],
            cluster_resource_whitelist: vec![],
            cluster_resource_blacklist: vec![],
            namespace_resource_whitelist: vec![],
            namespace_resource_blacklist: vec![],
            roles,
            sync_windows: vec![],
            orphaned_resources: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_rbac_allow_action() {
        let role = ProjectRole {
            name: "developer".to_string(),
            description: None,
            policies: vec![
                "p, role:developer, applications, sync, allow".to_string(),
                "p, role:developer, applications, get, allow".to_string(),
            ],
            groups: vec!["dev-team".to_string()],
        };
        let project = make_project(vec![role]);
        let subject = Subject {
            name: "alice".to_string(),
            groups: vec!["dev-team".to_string()],
        };
        assert!(check_permission(&subject, &project, "applications", "sync"));
        assert!(check_permission(&subject, &project, "applications", "get"));
    }

    #[test]
    fn test_rbac_deny_action() {
        let role = ProjectRole {
            name: "readonly".to_string(),
            description: None,
            policies: vec![
                "p, role:readonly, applications, get, allow".to_string(),
                "p, role:readonly, applications, delete, deny".to_string(),
            ],
            groups: vec!["viewers".to_string()],
        };
        let project = make_project(vec![role]);
        let subject = Subject { name: "bob".to_string(), groups: vec!["viewers".to_string()] };
        assert!(check_permission(&subject, &project, "applications", "get"));
        assert!(!check_permission(&subject, &project, "applications", "delete"));
    }

    #[test]
    fn test_rbac_wildcard_resource() {
        let role = ProjectRole {
            name: "admin".to_string(),
            description: None,
            policies: vec!["p, role:admin, *, *, allow".to_string()],
            groups: vec!["platform-team".to_string()],
        };
        let project = make_project(vec![role]);
        let subject =
            Subject { name: "carol".to_string(), groups: vec!["platform-team".to_string()] };
        assert!(check_permission(&subject, &project, "applications", "delete"));
        assert!(check_permission(&subject, &project, "clusters", "get"));
        assert!(check_permission(&subject, &project, "exec", "create"));
    }

    #[test]
    fn test_rbac_no_matching_role() {
        let role = ProjectRole {
            name: "developer".to_string(),
            description: None,
            policies: vec!["p, role:developer, applications, sync, allow".to_string()],
            groups: vec!["dev-team".to_string()],
        };
        let project = make_project(vec![role]);
        // Subject is not in any group and name doesn't match
        let subject = Subject { name: "unknown".to_string(), groups: vec![] };
        assert!(!check_permission(&subject, &project, "applications", "sync"));
    }

    #[test]
    fn test_rbac_explicit_deny_overrides_allow() {
        let roles = vec![
            ProjectRole {
                name: "broad".to_string(),
                description: None,
                policies: vec!["p, role:broad, *, *, allow".to_string()],
                groups: vec!["eng".to_string()],
            },
            ProjectRole {
                name: "restricted".to_string(),
                description: None,
                policies: vec!["p, role:restricted, clusters, delete, deny".to_string()],
                groups: vec!["eng".to_string()],
            },
        ];
        let project = make_project(roles);
        let subject = Subject { name: "dave".to_string(), groups: vec!["eng".to_string()] };
        // The deny in 'restricted' should block even though 'broad' allows everything
        assert!(!check_permission(&subject, &project, "clusters", "delete"));
        // But other actions still allowed
        assert!(check_permission(&subject, &project, "applications", "sync"));
    }
}

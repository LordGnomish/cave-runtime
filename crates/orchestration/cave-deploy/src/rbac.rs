// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Project-level RBAC — AppProject: sourceRepos, destinations,
//! clusterResourceWhitelist, roles, and RBAC policy evaluation.

use crate::models::Destination;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── AppProject CRD ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppProject {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub spec: AppProjectSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppProjectSpec {
    /// Allowed source repositories (glob patterns). "*" allows all.
    pub source_repos: Vec<String>,
    /// Allowed source namespaces (for ApplicationSets).
    #[serde(default)]
    pub source_namespaces: Vec<String>,
    /// Allowed deployment destinations.
    pub destinations: Vec<ProjectDestination>,
    /// Cluster resources this project may manage.
    #[serde(default)]
    pub cluster_resource_whitelist: Vec<GroupKind>,
    /// Cluster resources this project is forbidden from managing.
    #[serde(default)]
    pub cluster_resource_blacklist: Vec<GroupKind>,
    /// Namespace-scoped resources this project may manage.
    #[serde(default)]
    pub namespace_resource_whitelist: Vec<GroupKind>,
    /// Namespace-scoped resources this project is forbidden from managing.
    #[serde(default)]
    pub namespace_resource_blacklist: Vec<GroupKind>,
    /// Project-level roles.
    #[serde(default)]
    pub roles: Vec<ProjectRole>,
    /// Sync windows: restrict when syncs can occur.
    #[serde(default)]
    pub sync_windows: Vec<SyncWindow>,
    /// Signature keys for commit verification.
    #[serde(default)]
    pub signature_keys: Vec<SignatureKey>,
    /// Orphaned resources monitoring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orphaned_resources: Option<OrphanedResourcesMonitor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDestination {
    /// Cluster server URL or "*" for all clusters.
    pub server: String,
    /// Allowed namespaces (glob patterns). "*" allows all.
    pub namespace: String,
    /// Optional friendly name.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupKind {
    pub group: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRole {
    pub name: String,
    pub description: Option<String>,
    /// Casbin-style policy rules: "p, <role>, <resource>, <action>"
    pub policies: Vec<String>,
    pub jwt_tokens: Vec<JwtToken>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtToken {
    pub iat: i64,
    pub exp: Option<i64>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncWindow {
    pub kind: SyncWindowKind,
    pub schedule: String,
    pub duration: String,
    #[serde(default)]
    pub applications: Vec<String>,
    #[serde(default)]
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub clusters: Vec<String>,
    pub manual_sync: bool,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SyncWindowKind {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureKey {
    pub key_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanedResourcesMonitor {
    pub warn: bool,
    pub ignore: Vec<OrphanedResourceKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanedResourceKey {
    pub group: Option<String>,
    pub kind: Option<String>,
    pub name: Option<String>,
}

// ─── RBAC evaluation ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectViolation {
    SourceRepoNotAllowed { repo: String },
    DestinationNotAllowed { server: String, namespace: String },
    ClusterResourceNotAllowed { group: String, kind: String },
    NamespaceResourceBlacklisted { group: String, kind: String },
}

/// Check if a source repository is allowed by the project.
pub fn is_source_allowed(project: &AppProject, repo_url: &str) -> bool {
    project
        .spec
        .source_repos
        .iter()
        .any(|pattern| glob_match(pattern, repo_url))
}

/// Check if a destination is allowed by the project.
pub fn is_destination_allowed(project: &AppProject, dest: &Destination) -> bool {
    project.spec.destinations.iter().any(|d| {
        let server_ok = d.server == "*" || glob_match(&d.server, &dest.server);
        let ns_ok = d.namespace == "*" || glob_match(&d.namespace, &dest.namespace);
        server_ok && ns_ok
    })
}

/// Check if a cluster resource (non-namespaced) is allowed.
pub fn is_cluster_resource_allowed(project: &AppProject, group: &str, kind: &str) -> bool {
    // Blacklist takes priority
    if project
        .spec
        .cluster_resource_blacklist
        .iter()
        .any(|gk| (gk.group == "*" || gk.group == group) && (gk.kind == "*" || gk.kind == kind))
    {
        return false;
    }
    // Whitelist
    project
        .spec
        .cluster_resource_whitelist
        .iter()
        .any(|gk| (gk.group == "*" || gk.group == group) && (gk.kind == "*" || gk.kind == kind))
}

/// Check if a namespaced resource is allowed.
pub fn is_namespaced_resource_allowed(project: &AppProject, group: &str, kind: &str) -> bool {
    // Blacklist takes priority
    if project
        .spec
        .namespace_resource_blacklist
        .iter()
        .any(|gk| (gk.group == "*" || gk.group == group) && (gk.kind == "*" || gk.kind == kind))
    {
        return false;
    }
    // If whitelist is empty → allow all
    if project.spec.namespace_resource_whitelist.is_empty() {
        return true;
    }
    project
        .spec
        .namespace_resource_whitelist
        .iter()
        .any(|gk| (gk.group == "*" || gk.group == group) && (gk.kind == "*" || gk.kind == kind))
}

/// Validate all constraints for an application deployment.
pub fn validate_application(
    project: &AppProject,
    repo_url: &str,
    dest: &Destination,
) -> Vec<ProjectViolation> {
    let mut violations = Vec::new();

    if !is_source_allowed(project, repo_url) {
        violations.push(ProjectViolation::SourceRepoNotAllowed {
            repo: repo_url.to_string(),
        });
    }

    if !is_destination_allowed(project, dest) {
        violations.push(ProjectViolation::DestinationNotAllowed {
            server: dest.server.clone(),
            namespace: dest.namespace.clone(),
        });
    }

    violations
}

fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern == value {
        return true;
    }
    if pattern.ends_with("/*") {
        let prefix = &pattern[..pattern.len() - 2];
        return value.starts_with(prefix);
    }
    if pattern.contains('*') {
        // Simple wildcard split
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0;
        for part in &parts {
            if part.is_empty() {
                continue;
            }
            if let Some(idx) = value[pos..].find(part) {
                pos += idx + part.len();
            } else {
                return false;
            }
        }
        return true;
    }
    false
}

// ─── Role-based access control ───────────────────────────────────────────────

/// RBAC action on an application resource.
#[derive(Debug, Clone, PartialEq)]
pub enum RbacAction {
    Get,
    Create,
    Update,
    Delete,
    Sync,
    Override,
    Action(String),
}

/// Check if a subject (user/group) has permission to perform an action.
pub fn has_permission(
    project: &AppProject,
    subject: &str,
    resource: &str,
    action: &RbacAction,
) -> bool {
    let action_str = match action {
        RbacAction::Get => "get",
        RbacAction::Create => "create",
        RbacAction::Update => "update",
        RbacAction::Delete => "delete",
        RbacAction::Sync => "sync",
        RbacAction::Override => "override",
        RbacAction::Action(a) => a.as_str(),
    };

    for role in &project.spec.roles {
        let subject_in_role = role.groups.iter().any(|g| g == subject) || role.name == subject;
        if !subject_in_role {
            continue;
        }

        for policy in &role.policies {
            // Format: "p, role:name, resource, action, allow"
            let parts: Vec<&str> = policy.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 4 {
                let _effect = if parts.len() >= 5 { parts[4] } else { "allow" };
                let res_match =
                    parts[2] == "*" || parts[2] == resource || resource.starts_with(parts[2]);
                let act_match = parts[3] == "*" || parts[3] == action_str;
                if res_match && act_match {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Destination;

    fn make_project() -> AppProject {
        AppProject {
            id: Uuid::new_v4(),
            name: "my-project".to_string(),
            description: None,
            spec: AppProjectSpec {
                source_repos: vec![
                    "https://github.com/myorg/*".to_string(),
                    "https://charts.example.com".to_string(),
                ],
                source_namespaces: vec![],
                destinations: vec![
                    ProjectDestination {
                        server: "https://kubernetes.default.svc".to_string(),
                        namespace: "production".to_string(),
                        name: None,
                    },
                    ProjectDestination {
                        server: "https://kubernetes.default.svc".to_string(),
                        namespace: "staging".to_string(),
                        name: None,
                    },
                ],
                cluster_resource_whitelist: vec![GroupKind {
                    group: "".to_string(),
                    kind: "Namespace".to_string(),
                }],
                cluster_resource_blacklist: vec![],
                namespace_resource_whitelist: vec![],
                namespace_resource_blacklist: vec![GroupKind {
                    group: "".to_string(),
                    kind: "ResourceQuota".to_string(),
                }],
                roles: vec![ProjectRole {
                    name: "deploy-role".to_string(),
                    description: None,
                    policies: vec![
                        "p, deploy-role, applications, sync, allow".to_string(),
                        "p, deploy-role, applications, get, allow".to_string(),
                    ],
                    jwt_tokens: vec![],
                    groups: vec!["deploy-team".to_string()],
                }],
                sync_windows: vec![],
                signature_keys: vec![],
                orphaned_resources: None,
            },
        }
    }

    #[test]
    fn source_repo_allowed_glob() {
        let project = make_project();
        assert!(is_source_allowed(&project, "https://github.com/myorg/app"));
        assert!(is_source_allowed(&project, "https://charts.example.com"));
        assert!(!is_source_allowed(&project, "https://github.com/other/app"));
    }

    #[test]
    fn destination_allowed() {
        let project = make_project();
        let dest = Destination {
            server: "https://kubernetes.default.svc".to_string(),
            name: None,
            namespace: "production".to_string(),
        };
        assert!(is_destination_allowed(&project, &dest));
    }

    #[test]
    fn destination_not_allowed_namespace() {
        let project = make_project();
        let dest = Destination {
            server: "https://kubernetes.default.svc".to_string(),
            name: None,
            namespace: "kube-system".to_string(),
        };
        assert!(!is_destination_allowed(&project, &dest));
    }

    #[test]
    fn cluster_resource_allowed() {
        let project = make_project();
        assert!(is_cluster_resource_allowed(&project, "", "Namespace"));
        assert!(!is_cluster_resource_allowed(
            &project,
            "rbac.authorization.k8s.io",
            "ClusterRole"
        ));
    }

    #[test]
    fn namespace_resource_blacklist() {
        let project = make_project();
        assert!(!is_namespaced_resource_allowed(
            &project,
            "",
            "ResourceQuota"
        ));
        // Deployment not in blacklist, whitelist empty → allowed
        assert!(is_namespaced_resource_allowed(
            &project,
            "apps",
            "Deployment"
        ));
    }

    #[test]
    fn validate_application_source_violation() {
        let project = make_project();
        let dest = Destination {
            server: "https://kubernetes.default.svc".to_string(),
            name: None,
            namespace: "production".to_string(),
        };
        let violations = validate_application(&project, "https://evil.com/repo", &dest);
        assert_eq!(violations.len(), 1);
        assert!(matches!(
            violations[0],
            ProjectViolation::SourceRepoNotAllowed { .. }
        ));
    }

    #[test]
    fn validate_application_both_valid() {
        let project = make_project();
        let dest = Destination {
            server: "https://kubernetes.default.svc".to_string(),
            name: None,
            namespace: "staging".to_string(),
        };
        let violations = validate_application(&project, "https://github.com/myorg/backend", &dest);
        assert!(violations.is_empty());
    }

    #[test]
    fn has_permission_sync() {
        let project = make_project();
        assert!(has_permission(
            &project,
            "deploy-team",
            "applications",
            &RbacAction::Sync
        ));
        assert!(has_permission(
            &project,
            "deploy-team",
            "applications",
            &RbacAction::Get
        ));
    }

    #[test]
    fn has_permission_denied() {
        let project = make_project();
        assert!(!has_permission(
            &project,
            "deploy-team",
            "applications",
            &RbacAction::Delete
        ));
        assert!(!has_permission(
            &project,
            "other-team",
            "applications",
            &RbacAction::Sync
        ));
    }

    #[test]
    fn glob_match_wildcard() {
        assert!(glob_match(
            "https://github.com/myorg/*",
            "https://github.com/myorg/app"
        ));
        assert!(!glob_match(
            "https://github.com/myorg/*",
            "https://github.com/other/app"
        ));
    }
}

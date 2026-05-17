// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RBAC bootstrap — admin, developer, and viewer roles + bindings.

use crate::error::{ClusterError, ClusterResult};
use serde::{Deserialize, Serialize};

/// CAVE platform role mapped to K8s ClusterRole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformRole {
    Admin,
    Developer,
    Viewer,
}

impl PlatformRole {
    pub fn cluster_role_name(&self) -> &'static str {
        match self {
            Self::Admin => "cave:cluster-admin",
            Self::Developer => "cave:developer",
            Self::Viewer => "cave:viewer",
        }
    }
}

/// A K8s RBAC PolicyRule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
}

/// A K8s ClusterRole.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRole {
    pub name: String,
    pub labels: std::collections::HashMap<String, String>,
    pub rules: Vec<PolicyRule>,
}

/// A K8s ClusterRoleBinding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRoleBinding {
    pub name: String,
    pub role_ref: String,
    pub subjects: Vec<RbacSubject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbacSubject {
    pub kind: String, // "User", "Group", "ServiceAccount"
    pub name: String,
    pub namespace: Option<String>,
}

/// A RoleBinding in a specific namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBinding {
    pub name: String,
    pub namespace: String,
    pub role_ref: String,
    pub subjects: Vec<RbacSubject>,
}

/// The full set of RBAC objects to bootstrap a cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbacBootstrap {
    pub cluster_roles: Vec<ClusterRole>,
    pub cluster_role_bindings: Vec<ClusterRoleBinding>,
    pub role_bindings: Vec<RoleBinding>,
}

/// Generate the default RBAC objects for a new cluster.
pub fn default_bootstrap(cluster_name: &str) -> RbacBootstrap {
    let mut labels = std::collections::HashMap::new();
    labels.insert("cave.io/cluster".into(), cluster_name.to_string());
    labels.insert("cave.io/managed-by".into(), "cave-cluster".into());

    let admin_role = ClusterRole {
        name: "cave:cluster-admin".into(),
        labels: labels.clone(),
        rules: vec![PolicyRule {
            api_groups: vec!["*".into()],
            resources: vec!["*".into()],
            verbs: vec!["*".into()],
        }],
    };

    let developer_role = ClusterRole {
        name: "cave:developer".into(),
        labels: labels.clone(),
        rules: vec![
            PolicyRule {
                api_groups: vec!["".into()],
                resources: vec![
                    "pods".into(), "services".into(), "configmaps".into(),
                    "persistentvolumeclaims".into(), "events".into(), "secrets".into(),
                ],
                verbs: vec![
                    "get".into(), "list".into(), "watch".into(),
                    "create".into(), "update".into(), "patch".into(), "delete".into(),
                ],
            },
            PolicyRule {
                api_groups: vec!["apps".into()],
                resources: vec![
                    "deployments".into(), "replicasets".into(), "statefulsets".into(),
                    "daemonsets".into(),
                ],
                verbs: vec![
                    "get".into(), "list".into(), "watch".into(),
                    "create".into(), "update".into(), "patch".into(), "delete".into(),
                ],
            },
            PolicyRule {
                api_groups: vec!["batch".into()],
                resources: vec!["jobs".into(), "cronjobs".into()],
                verbs: vec![
                    "get".into(), "list".into(), "watch".into(),
                    "create".into(), "update".into(), "patch".into(), "delete".into(),
                ],
            },
            PolicyRule {
                api_groups: vec!["networking.k8s.io".into()],
                resources: vec!["ingresses".into(), "networkpolicies".into()],
                verbs: vec!["get".into(), "list".into(), "watch".into(), "create".into(), "update".into()],
            },
        ],
    };

    let viewer_role = ClusterRole {
        name: "cave:viewer".into(),
        labels: labels.clone(),
        rules: vec![
            PolicyRule {
                api_groups: vec!["".into(), "apps".into(), "batch".into(), "networking.k8s.io".into()],
                resources: vec!["*".into()],
                verbs: vec!["get".into(), "list".into(), "watch".into()],
            },
        ],
    };

    RbacBootstrap {
        cluster_roles: vec![admin_role, developer_role, viewer_role],
        cluster_role_bindings: vec![],
        role_bindings: vec![],
    }
}

/// Generate a kubeconfig token RBAC binding for a principal.
pub fn bind_principal(
    principal: &str,
    role: PlatformRole,
    namespace: Option<&str>,
) -> ClusterResult<(Option<ClusterRoleBinding>, Option<RoleBinding>)> {
    let subject = RbacSubject {
        kind: "User".into(),
        name: principal.to_string(),
        namespace: None,
    };
    match namespace {
        None => Ok((
            Some(ClusterRoleBinding {
                name: format!("cave:{}-{}", role.cluster_role_name(), principal),
                role_ref: role.cluster_role_name().to_string(),
                subjects: vec![subject],
            }),
            None,
        )),
        Some(ns) => Ok((
            None,
            Some(RoleBinding {
                name: format!("cave:{}-{}", role.cluster_role_name(), principal),
                namespace: ns.to_string(),
                role_ref: role.cluster_role_name().to_string(),
                subjects: vec![subject],
            }),
        )),
    }
}

/// Convert RbacBootstrap to a series of K8s YAML manifests.
pub fn to_yaml_manifests(bootstrap: &RbacBootstrap) -> Vec<String> {
    let mut manifests = Vec::new();

    for cr in &bootstrap.cluster_roles {
        let rules_yaml: String = cr.rules.iter().map(|r| {
            format!(
                "  - apiGroups: {}\n    resources: {}\n    verbs: {}",
                serde_json::to_string(&r.api_groups).unwrap_or_default(),
                serde_json::to_string(&r.resources).unwrap_or_default(),
                serde_json::to_string(&r.verbs).unwrap_or_default(),
            )
        }).collect::<Vec<_>>().join("\n");

        manifests.push(format!(
            r#"apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {name}
rules:
{rules}"#,
            name = cr.name,
            rules = rules_yaml,
        ));
    }

    for crb in &bootstrap.cluster_role_bindings {
        let subjects_yaml: String = crb.subjects.iter().map(|s| {
            format!("  - kind: {}\n    name: {}", s.kind, s.name)
        }).collect::<Vec<_>>().join("\n");

        manifests.push(format!(
            r#"apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: {name}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: {role}
subjects:
{subjects}"#,
            name = crb.name,
            role = crb.role_ref,
            subjects = subjects_yaml,
        ));
    }

    manifests
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bootstrap_has_three_roles() {
        let bootstrap = default_bootstrap("test-cluster");
        assert_eq!(bootstrap.cluster_roles.len(), 3);
        let names: Vec<&str> = bootstrap.cluster_roles.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"cave:cluster-admin"));
        assert!(names.contains(&"cave:developer"));
        assert!(names.contains(&"cave:viewer"));
    }

    #[test]
    fn bind_principal_cluster_wide() {
        let (crb, rb) = bind_principal("alice@example.com", PlatformRole::Admin, None).unwrap();
        assert!(crb.is_some());
        assert!(rb.is_none());
        let crb = crb.unwrap();
        assert_eq!(crb.role_ref, "cave:cluster-admin");
        assert_eq!(crb.subjects[0].name, "alice@example.com");
    }

    #[test]
    fn bind_principal_namespace_scoped() {
        let (crb, rb) = bind_principal("bob", PlatformRole::Developer, Some("production")).unwrap();
        assert!(crb.is_none());
        assert!(rb.is_some());
        let rb = rb.unwrap();
        assert_eq!(rb.namespace, "production");
    }

    #[test]
    fn to_yaml_manifests_non_empty() {
        let bootstrap = default_bootstrap("my-cluster");
        let manifests = to_yaml_manifests(&bootstrap);
        assert!(!manifests.is_empty());
        assert!(manifests[0].contains("ClusterRole"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Umbrella-level model types shared across cave-k8s facade modules.
//!
//! Resource-level types (`Pod`, `Service`, `Deployment` …) live in
//! `cave-apiserver`. The structures in this module describe the
//! *control-plane* shape — cluster phase, component health, generic
//! resource references — and bind the eight subsystems together.

use serde::{Deserialize, Serialize};

/// The eight named control-plane components managed by cave-k8s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentName {
    Apiserver,
    Scheduler,
    ControllerManager,
    CloudControllerManager,
    Kubelet,
    KubeProxy,
    Etcd,
    Cri,
}

impl ComponentName {
    pub const ALL: [ComponentName; 8] = [
        ComponentName::Apiserver,
        ComponentName::Scheduler,
        ComponentName::ControllerManager,
        ComponentName::CloudControllerManager,
        ComponentName::Kubelet,
        ComponentName::KubeProxy,
        ComponentName::Etcd,
        ComponentName::Cri,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ComponentName::Apiserver => "apiserver",
            ComponentName::Scheduler => "scheduler",
            ComponentName::ControllerManager => "controller-manager",
            ComponentName::CloudControllerManager => "cloud-controller-manager",
            ComponentName::Kubelet => "kubelet",
            ComponentName::KubeProxy => "kube-proxy",
            ComponentName::Etcd => "etcd",
            ComponentName::Cri => "cri",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentHealth {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// High-level cluster phase, similar to K8s `Cluster.status.phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ClusterPhase {
    Pending,
    Bootstrapping,
    Running,
    Draining,
    Failed,
}

/// Node taxonomy.  Cave Runtime nodes follow K8s' control-plane / worker
/// split; `Hybrid` carries both roles for single-node clusters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeRole {
    ControlPlane,
    Worker,
    Hybrid,
}

/// All Kubernetes built-in resource kinds that cave-k8s coordinates.
/// Out-of-tree CRDs go through `crd::CrdRegistry`, not this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BuiltinKind {
    Namespace,
    Node,
    Pod,
    Service,
    ConfigMap,
    Secret,
    PersistentVolume,
    PersistentVolumeClaim,
    StorageClass,
    Deployment,
    ReplicaSet,
    StatefulSet,
    DaemonSet,
    Job,
    CronJob,
    Endpoints,
    EndpointSlice,
    Ingress,
    ServiceAccount,
    Role,
    RoleBinding,
    ClusterRole,
    ClusterRoleBinding,
    Event,
}

impl BuiltinKind {
    /// True when the kind is scoped to a namespace.  Non-namespaced kinds
    /// (`Namespace`, `Node`, `PV`, `StorageClass`, `ClusterRole*`) live in
    /// the cluster scope.
    pub fn is_namespaced(self) -> bool {
        !matches!(
            self,
            BuiltinKind::Namespace
                | BuiltinKind::Node
                | BuiltinKind::PersistentVolume
                | BuiltinKind::StorageClass
                | BuiltinKind::ClusterRole
                | BuiltinKind::ClusterRoleBinding
        )
    }

    pub fn api_group(self) -> &'static str {
        match self {
            BuiltinKind::Deployment
            | BuiltinKind::ReplicaSet
            | BuiltinKind::StatefulSet
            | BuiltinKind::DaemonSet => "apps",
            BuiltinKind::Job | BuiltinKind::CronJob => "batch",
            BuiltinKind::EndpointSlice => "discovery.k8s.io",
            BuiltinKind::Ingress => "networking.k8s.io",
            BuiltinKind::Role
            | BuiltinKind::RoleBinding
            | BuiltinKind::ClusterRole
            | BuiltinKind::ClusterRoleBinding => "rbac.authorization.k8s.io",
            BuiltinKind::StorageClass => "storage.k8s.io",
            _ => "",
        }
    }

    pub fn api_version(self) -> &'static str {
        match self {
            BuiltinKind::Job | BuiltinKind::CronJob => "v1",
            BuiltinKind::Ingress => "v1",
            BuiltinKind::EndpointSlice => "v1",
            _ => "v1",
        }
    }
}

/// A typed reference to a resource — `(group, kind, namespace, name)`.
/// Matches K8s' `OwnerReference` shape closely enough that
/// `garbage_collector` can use it as a cascade-delete edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResourceRef {
    pub group: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}

impl ResourceRef {
    pub fn cluster_scoped(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            group: String::new(),
            kind: kind.into(),
            namespace: None,
            name: name.into(),
        }
    }

    pub fn namespaced(
        kind: impl Into<String>,
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            group: String::new(),
            kind: kind.into(),
            namespace: Some(namespace.into()),
            name: name.into(),
        }
    }

    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_all_covers_eight_subsystems() {
        assert_eq!(ComponentName::ALL.len(), 8);
        let names: Vec<&str> = ComponentName::ALL.iter().map(|c| c.as_str()).collect();
        assert!(names.contains(&"apiserver"));
        assert!(names.contains(&"etcd"));
        assert!(names.contains(&"cri"));
    }

    #[test]
    fn namespaced_distinction_matches_k8s() {
        assert!(BuiltinKind::Pod.is_namespaced());
        assert!(BuiltinKind::Service.is_namespaced());
        assert!(BuiltinKind::Deployment.is_namespaced());
        assert!(BuiltinKind::Role.is_namespaced());
        assert!(!BuiltinKind::Namespace.is_namespaced());
        assert!(!BuiltinKind::Node.is_namespaced());
        assert!(!BuiltinKind::PersistentVolume.is_namespaced());
        assert!(!BuiltinKind::StorageClass.is_namespaced());
        assert!(!BuiltinKind::ClusterRole.is_namespaced());
    }

    #[test]
    fn api_groups_pin_to_k8s_groups() {
        assert_eq!(BuiltinKind::Deployment.api_group(), "apps");
        assert_eq!(BuiltinKind::Job.api_group(), "batch");
        assert_eq!(BuiltinKind::Ingress.api_group(), "networking.k8s.io");
        assert_eq!(
            BuiltinKind::ClusterRole.api_group(),
            "rbac.authorization.k8s.io"
        );
        assert_eq!(BuiltinKind::StorageClass.api_group(), "storage.k8s.io");
        assert_eq!(BuiltinKind::Pod.api_group(), "");
    }

    #[test]
    fn ref_constructors_capture_scope() {
        let c = ResourceRef::cluster_scoped("Node", "n1");
        assert!(c.namespace.is_none());
        assert_eq!(c.kind, "Node");
        let n = ResourceRef::namespaced("Pod", "default", "p1");
        assert_eq!(n.namespace.as_deref(), Some("default"));
        let g = ResourceRef::namespaced("Deployment", "default", "d1").with_group("apps");
        assert_eq!(g.group, "apps");
    }

    #[test]
    fn component_serialization_is_kebab_case() {
        let s = serde_json::to_string(&ComponentName::CloudControllerManager).unwrap();
        assert_eq!(s, "\"cloud-controller-manager\"");
    }

    #[test]
    fn cluster_phase_serializes_pascal_case() {
        let s = serde_json::to_string(&ClusterPhase::Bootstrapping).unwrap();
        assert_eq!(s, "\"Bootstrapping\"");
    }
}

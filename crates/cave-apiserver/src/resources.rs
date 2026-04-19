//! Kubernetes resource types — Pod, Deployment, Service, ConfigMap, Secret, Namespace.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Common metadata for all K8s resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMeta {
    pub name: String,
    pub namespace: String,
    pub uid: Uuid,
    pub resource_version: u64,
    pub creation_timestamp: DateTime<Utc>,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub owner_references: Vec<OwnerReference>,
    pub finalizers: Vec<String>,
    pub deletion_timestamp: Option<DateTime<Utc>>,
}

impl ObjectMeta {
    pub fn new(name: &str, namespace: &str) -> Self {
        Self {
            name: name.to_string(),
            namespace: namespace.to_string(),
            uid: Uuid::new_v4(),
            resource_version: 1,
            creation_timestamp: Utc::now(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            owner_references: vec![],
            finalizers: vec![],
            deletion_timestamp: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerReference {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub uid: Uuid,
}

/// Pod resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pod {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: PodSpec,
    pub status: PodStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSpec {
    pub containers: Vec<ContainerDef>,
    pub init_containers: Vec<ContainerDef>,
    pub restart_policy: String,
    pub service_account_name: Option<String>,
    pub node_name: Option<String>,
    pub node_selector: HashMap<String, String>,
    pub tolerations: Vec<Toleration>,
    pub volumes: Vec<Volume>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerDef {
    pub name: String,
    pub image: String,
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    pub ports: Vec<ContainerPort>,
    pub resources: ResourceRequirements,
    pub volume_mounts: Vec<VolumeMount>,
    pub liveness_probe: Option<Probe>,
    pub readiness_probe: Option<Probe>,
    pub image_pull_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: Option<String>,
    pub value_from: Option<EnvVarSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarSource {
    pub config_map_key_ref: Option<KeyRef>,
    pub secret_key_ref: Option<KeyRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRef {
    pub name: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPort {
    pub name: Option<String>,
    pub container_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub requests: HashMap<String, String>,
    pub limits: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub name: String,
    pub mount_path: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub http_get: Option<HttpGetAction>,
    pub exec: Option<ExecAction>,
    pub tcp_socket: Option<TcpSocketAction>,
    pub initial_delay_seconds: u32,
    pub period_seconds: u32,
    pub timeout_seconds: u32,
    pub failure_threshold: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpGetAction { pub path: String, pub port: u16 }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecAction { pub command: Vec<String> }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpSocketAction { pub port: u16 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Toleration {
    pub key: Option<String>,
    pub operator: String,
    pub value: Option<String>,
    pub effect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub name: String,
    pub config_map: Option<ConfigMapVolumeSource>,
    pub secret: Option<SecretVolumeSource>,
    pub empty_dir: Option<EmptyDirVolumeSource>,
    pub host_path: Option<HostPathVolumeSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMapVolumeSource { pub name: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretVolumeSource { pub secret_name: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyDirVolumeSource { pub medium: Option<String> }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPathVolumeSource { pub path: String }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PodStatus {
    pub phase: String,
    pub conditions: Vec<PodCondition>,
    pub host_ip: Option<String>,
    pub pod_ip: Option<String>,
    pub container_statuses: Vec<ContainerStatus>,
    pub start_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodCondition {
    pub condition_type: String,
    pub status: String,
    pub last_transition_time: DateTime<Utc>,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub name: String,
    pub ready: bool,
    pub restart_count: u32,
    pub image: String,
    pub started: bool,
}

/// Deployment resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: DeploymentSpec,
    pub status: DeploymentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentSpec {
    pub replicas: u32,
    pub selector: LabelSelector,
    pub template: PodTemplateSpec,
    pub strategy: DeploymentStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelSelector {
    pub match_labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodTemplateSpec {
    pub metadata: ObjectMeta,
    pub spec: PodSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentStrategy {
    pub strategy_type: String,
    pub rolling_update: Option<RollingUpdateDeployment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollingUpdateDeployment {
    pub max_unavailable: String,
    pub max_surge: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeploymentStatus {
    pub replicas: u32,
    pub ready_replicas: u32,
    pub available_replicas: u32,
    pub updated_replicas: u32,
    pub conditions: Vec<DeploymentCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentCondition {
    pub condition_type: String,
    pub status: String,
    pub last_transition_time: DateTime<Utc>,
    pub reason: String,
    pub message: String,
}

/// Service resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: ServiceSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub service_type: String,
    pub selector: HashMap<String, String>,
    pub ports: Vec<ServicePort>,
    pub cluster_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: Option<String>,
    pub port: u16,
    pub target_port: u16,
    pub protocol: String,
}

/// ConfigMap resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMap {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub data: HashMap<String, String>,
}

/// Secret resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub data: HashMap<String, String>,
    pub secret_type: String,
}

/// Namespace resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub status: NamespaceStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamespaceStatus {
    pub phase: String,
}

/// Generic resource wrapper for the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Resource {
    Pod(Pod),
    Deployment(Deployment),
    Service(Service),
    ConfigMap(ConfigMap),
    Secret(Secret),
    Namespace(Namespace),
}

impl Resource {
    pub fn kind(&self) -> &str {
        match self {
            Resource::Pod(_) => "Pod",
            Resource::Deployment(_) => "Deployment",
            Resource::Service(_) => "Service",
            Resource::ConfigMap(_) => "ConfigMap",
            Resource::Secret(_) => "Secret",
            Resource::Namespace(_) => "Namespace",
        }
    }

    pub fn metadata(&self) -> &ObjectMeta {
        match self {
            Resource::Pod(r) => &r.metadata,
            Resource::Deployment(r) => &r.metadata,
            Resource::Service(r) => &r.metadata,
            Resource::ConfigMap(r) => &r.metadata,
            Resource::Secret(r) => &r.metadata,
            Resource::Namespace(r) => &r.metadata,
        }
    }

    pub fn name(&self) -> &str { &self.metadata().name }
    pub fn namespace(&self) -> &str { &self.metadata().namespace }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_meta_new() {
        let m = ObjectMeta::new("nginx", "default");
        assert_eq!(m.name, "nginx");
        assert_eq!(m.namespace, "default");
        assert_eq!(m.resource_version, 1);
    }

    #[test]
    fn test_resource_kind() {
        let cm = Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            metadata: ObjectMeta::new("test", "default"),
            data: HashMap::new(),
        });
        assert_eq!(cm.kind(), "ConfigMap");
        assert_eq!(cm.name(), "test");
    }
}

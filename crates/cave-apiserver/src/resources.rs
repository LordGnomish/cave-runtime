//! Kubernetes resource types — full core API surface parity.

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectReference {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub api_version: Option<String>,
    pub uid: Option<Uuid>,
}

impl Default for ObjectReference {
    fn default() -> Self {
        Self { kind: String::new(), name: String::new(), namespace: String::new(), api_version: None, uid: None }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalObjectReference {
    pub name: String,
}

// ── Shared selector ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelSelector {
    pub match_labels: HashMap<String, String>,
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: String,
    pub values: Vec<String>,
}

// ── Pod / Container primitives ───────────────────────────────────────────────

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

impl Default for PodSpec {
    fn default() -> Self {
        Self {
            containers: vec![],
            init_containers: vec![],
            restart_policy: "Always".into(),
            service_account_name: None,
            node_name: None,
            node_selector: HashMap::new(),
            tolerations: vec![],
            volumes: vec![],
        }
    }
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
    pub persistent_volume_claim: Option<PersistentVolumeClaimVolumeSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMapVolumeSource { pub name: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretVolumeSource { pub secret_name: String }
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmptyDirVolumeSource { pub medium: Option<String> }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPathVolumeSource { pub path: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolumeClaimVolumeSource { pub claim_name: String, pub read_only: bool }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodTemplateSpec {
    pub metadata: ObjectMeta,
    pub spec: PodSpec,
}

impl Default for PodTemplateSpec {
    fn default() -> Self {
        Self { metadata: ObjectMeta::new("", ""), spec: PodSpec::default() }
    }
}

// ── Deployment ───────────────────────────────────────────────────────────────

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

impl Default for DeploymentSpec {
    fn default() -> Self {
        Self {
            replicas: 1,
            selector: LabelSelector::default(),
            template: PodTemplateSpec::default(),
            strategy: DeploymentStrategy { strategy_type: "RollingUpdate".into(), rolling_update: None },
        }
    }
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

// ── Scale (status subresource) ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scale {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: ScaleSpec,
    pub status: ScaleStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScaleSpec { pub replicas: u32 }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScaleStatus { pub replicas: u32, pub selector: Option<String> }

// ── StatefulSet ───────────────────────────────────────────────────────────────

/// StatefulSet resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatefulSet {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: StatefulSetSpec,
    pub status: StatefulSetStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatefulSetSpec {
    pub replicas: u32,
    pub selector: LabelSelector,
    pub template: PodTemplateSpec,
    pub service_name: String,
    pub volume_claim_templates: Vec<PersistentVolumeClaim>,
}

impl Default for StatefulSetSpec {
    fn default() -> Self {
        Self {
            replicas: 1,
            selector: LabelSelector::default(),
            template: PodTemplateSpec::default(),
            service_name: String::new(),
            volume_claim_templates: vec![],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatefulSetStatus {
    pub replicas: u32,
    pub ready_replicas: u32,
    pub current_replicas: u32,
    pub updated_replicas: u32,
    pub current_revision: String,
    pub update_revision: String,
}

// ── DaemonSet ─────────────────────────────────────────────────────────────────

/// DaemonSet resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSet {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: DaemonSetSpec,
    pub status: DaemonSetStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSetSpec {
    pub selector: LabelSelector,
    pub template: PodTemplateSpec,
    pub update_strategy: DaemonSetUpdateStrategy,
}

impl Default for DaemonSetSpec {
    fn default() -> Self {
        Self {
            selector: LabelSelector::default(),
            template: PodTemplateSpec::default(),
            update_strategy: DaemonSetUpdateStrategy { update_strategy_type: "RollingUpdate".into() },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSetUpdateStrategy { pub update_strategy_type: String }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonSetStatus {
    pub desired_number_scheduled: u32,
    pub current_number_scheduled: u32,
    pub number_ready: u32,
    pub number_available: u32,
    pub number_unavailable: u32,
}

// ── ReplicaSet ────────────────────────────────────────────────────────────────

/// ReplicaSet resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaSet {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: ReplicaSetSpec,
    pub status: ReplicaSetStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaSetSpec {
    pub replicas: u32,
    pub selector: LabelSelector,
    pub template: PodTemplateSpec,
}

impl Default for ReplicaSetSpec {
    fn default() -> Self {
        Self { replicas: 1, selector: LabelSelector::default(), template: PodTemplateSpec::default() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplicaSetStatus {
    pub replicas: u32,
    pub ready_replicas: u32,
    pub available_replicas: u32,
}

// ── Job / CronJob ─────────────────────────────────────────────────────────────

/// Job resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: JobSpec,
    pub status: JobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub template: PodTemplateSpec,
    pub completions: Option<u32>,
    pub parallelism: Option<u32>,
    pub backoff_limit: Option<u32>,
    pub active_deadline_seconds: Option<u64>,
}

impl Default for JobSpec {
    fn default() -> Self {
        Self {
            template: PodTemplateSpec::default(),
            completions: Some(1),
            parallelism: Some(1),
            backoff_limit: Some(6),
            active_deadline_seconds: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStatus {
    pub active: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub completion_time: Option<DateTime<Utc>>,
    pub start_time: Option<DateTime<Utc>>,
}

/// CronJob resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: CronJobSpec,
    pub status: CronJobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobSpec {
    pub schedule: String,
    pub job_template: JobTemplateSpec,
    pub concurrency_policy: String,
    pub suspend: bool,
    pub successful_jobs_history_limit: Option<u32>,
    pub failed_jobs_history_limit: Option<u32>,
}

impl Default for CronJobSpec {
    fn default() -> Self {
        Self {
            schedule: "0 * * * *".into(),
            job_template: JobTemplateSpec::default(),
            concurrency_policy: "Allow".into(),
            suspend: false,
            successful_jobs_history_limit: Some(3),
            failed_jobs_history_limit: Some(1),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTemplateSpec {
    pub metadata: ObjectMeta,
    pub spec: JobSpec,
}

impl Default for JobTemplateSpec {
    fn default() -> Self {
        Self { metadata: ObjectMeta::new("", ""), spec: JobSpec::default() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronJobStatus {
    pub active: Vec<ObjectReference>,
    pub last_schedule_time: Option<DateTime<Utc>>,
    pub last_successful_time: Option<DateTime<Utc>>,
}

// ── Ingress ───────────────────────────────────────────────────────────────────

/// Ingress resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ingress {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: IngressSpec,
    pub status: IngressStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngressSpec {
    pub ingress_class_name: Option<String>,
    pub rules: Vec<IngressRule>,
    pub tls: Vec<IngressTLS>,
    pub default_backend: Option<IngressBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressRule {
    pub host: Option<String>,
    pub http: Option<HTTPIngressRuleValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HTTPIngressRuleValue { pub paths: Vec<HTTPIngressPath> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HTTPIngressPath {
    pub path: String,
    pub path_type: String,
    pub backend: IngressBackend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressBackend {
    pub service: IngressServiceBackend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressServiceBackend {
    pub name: String,
    pub port: ServiceBackendPort,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceBackendPort { pub number: u16, pub name: Option<String> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngressTLS { pub hosts: Vec<String>, pub secret_name: Option<String> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngressStatus {
    pub load_balancer: IngressLoadBalancerStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngressLoadBalancerStatus { pub ingress: Vec<IngressLoadBalancerIngress> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngressLoadBalancerIngress { pub ip: Option<String>, pub hostname: Option<String> }

// ── NetworkPolicy ─────────────────────────────────────────────────────────────

/// NetworkPolicy resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: NetworkPolicySpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPolicySpec {
    pub pod_selector: LabelSelector,
    pub ingress: Vec<NetworkPolicyIngressRule>,
    pub egress: Vec<NetworkPolicyEgressRule>,
    pub policy_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPolicyIngressRule {
    pub from: Vec<NetworkPolicyPeer>,
    pub ports: Vec<NetworkPolicyPort>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPolicyEgressRule {
    pub to: Vec<NetworkPolicyPeer>,
    pub ports: Vec<NetworkPolicyPort>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPolicyPeer {
    pub pod_selector: Option<LabelSelector>,
    pub namespace_selector: Option<LabelSelector>,
    pub ip_block: Option<IPBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPBlock { pub cidr: String, pub except: Vec<String> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkPolicyPort { pub port: Option<u16>, pub protocol: Option<String> }

// ── Storage ───────────────────────────────────────────────────────────────────

/// PersistentVolume resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolume {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: PersistentVolumeSpec,
    pub status: PersistentVolumeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolumeSpec {
    pub capacity: HashMap<String, String>,
    pub access_modes: Vec<String>,
    pub reclaim_policy: String,
    pub storage_class_name: Option<String>,
    pub volume_mode: Option<String>,
    pub claim_ref: Option<ObjectReference>,
}

impl Default for PersistentVolumeSpec {
    fn default() -> Self {
        Self {
            capacity: HashMap::new(),
            access_modes: vec!["ReadWriteOnce".into()],
            reclaim_policy: "Retain".into(),
            storage_class_name: None,
            volume_mode: Some("Filesystem".into()),
            claim_ref: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolumeStatus {
    pub phase: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

impl Default for PersistentVolumeStatus {
    fn default() -> Self {
        Self { phase: "Available".into(), reason: None, message: None }
    }
}

/// PersistentVolumeClaim resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolumeClaim {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: PersistentVolumeClaimSpec,
    pub status: PersistentVolumeClaimStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolumeClaimSpec {
    pub access_modes: Vec<String>,
    pub resources: ResourceRequirements,
    pub storage_class_name: Option<String>,
    pub volume_name: Option<String>,
    pub volume_mode: Option<String>,
}

impl Default for PersistentVolumeClaimSpec {
    fn default() -> Self {
        Self {
            access_modes: vec!["ReadWriteOnce".into()],
            resources: ResourceRequirements::default(),
            storage_class_name: None,
            volume_name: None,
            volume_mode: Some("Filesystem".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolumeClaimStatus {
    pub phase: String,
    pub access_modes: Vec<String>,
    pub capacity: HashMap<String, String>,
}

impl Default for PersistentVolumeClaimStatus {
    fn default() -> Self {
        Self { phase: "Pending".into(), access_modes: vec![], capacity: HashMap::new() }
    }
}

/// StorageClass resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageClass {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub provisioner: String,
    pub parameters: HashMap<String, String>,
    pub reclaim_policy: Option<String>,
    pub volume_binding_mode: Option<String>,
    pub allow_volume_expansion: bool,
}

// ── RBAC ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyRule {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
    pub resource_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleRef {
    pub api_group: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subject {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub api_group: Option<String>,
}

/// Role resource (namespaced).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub rules: Vec<PolicyRule>,
}

/// ClusterRole resource (cluster-scoped).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRole {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub rules: Vec<PolicyRule>,
    pub aggregation_rule: Option<AggregationRule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregationRule { pub cluster_role_selectors: Vec<LabelSelector> }

/// RoleBinding resource (namespaced).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBinding {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub subjects: Vec<Subject>,
    pub role_ref: RoleRef,
}

/// ClusterRoleBinding resource (cluster-scoped).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRoleBinding {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub subjects: Vec<Subject>,
    pub role_ref: RoleRef,
}

// ── Core v1 resources ─────────────────────────────────────────────────────────

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
pub struct NamespaceStatus { pub phase: String }

/// ServiceAccount resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAccount {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub secrets: Vec<ObjectReference>,
    pub image_pull_secrets: Vec<LocalObjectReference>,
    pub automount_service_account_token: Option<bool>,
}

/// Node resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: NodeSpec,
    pub status: NodeStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeSpec {
    pub pod_cidr: Option<String>,
    pub provider_id: Option<String>,
    pub unschedulable: bool,
    pub taints: Vec<Taint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: Option<String>,
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    pub capacity: HashMap<String, String>,
    pub allocatable: HashMap<String, String>,
    pub conditions: Vec<NodeCondition>,
    pub addresses: Vec<NodeAddress>,
    pub node_info: NodeSystemInfo,
}

impl Default for NodeStatus {
    fn default() -> Self {
        Self {
            capacity: HashMap::new(),
            allocatable: HashMap::new(),
            conditions: vec![],
            addresses: vec![],
            node_info: NodeSystemInfo::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCondition {
    pub condition_type: String,
    pub status: String,
    pub last_transition_time: DateTime<Utc>,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeAddress { pub address_type: String, pub address: String }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeSystemInfo {
    pub machine_id: String,
    pub kernel_version: String,
    pub os_image: String,
    pub container_runtime_version: String,
    pub kubelet_version: String,
    pub architecture: String,
    pub operating_system: String,
}

/// Event resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeEvent {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub involved_object: ObjectReference,
    pub reason: String,
    pub message: String,
    pub event_type: String,
    pub count: u32,
    pub first_timestamp: Option<DateTime<Utc>>,
    pub last_timestamp: Option<DateTime<Utc>>,
    pub source: EventSource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventSource { pub component: String, pub host: Option<String> }

/// Endpoints resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoints {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub subsets: Vec<EndpointSubset>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointSubset {
    pub addresses: Vec<EndpointAddress>,
    pub not_ready_addresses: Vec<EndpointAddress>,
    pub ports: Vec<EndpointPort>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointAddress {
    pub ip: String,
    pub hostname: Option<String>,
    pub target_ref: Option<ObjectReference>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointPort { pub name: Option<String>, pub port: u16, pub protocol: String }

/// ResourceQuota resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuota {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: ResourceQuotaSpec,
    pub status: ResourceQuotaStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceQuotaSpec {
    pub hard: HashMap<String, String>,
    pub scope_selector: Option<ScopeSelector>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScopeSelector { pub match_expressions: Vec<ScopedResourceSelectorRequirement> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScopedResourceSelectorRequirement {
    pub scope_name: String,
    pub operator: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceQuotaStatus {
    pub hard: HashMap<String, String>,
    pub used: HashMap<String, String>,
}

/// LimitRange resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitRange {
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: LimitRangeSpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LimitRangeSpec { pub limits: Vec<LimitRangeItem> }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LimitRangeItem {
    pub limit_type: String,
    pub max: HashMap<String, String>,
    pub min: HashMap<String, String>,
    pub default: HashMap<String, String>,
    pub default_request: HashMap<String, String>,
}

// ── Generic resource wrapper ──────────────────────────────────────────────────

/// All resource types as a single enum for the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Resource {
    // Core v1 — namespaced
    Pod(Pod),
    Service(Service),
    ConfigMap(ConfigMap),
    Secret(Secret),
    ServiceAccount(ServiceAccount),
    KubeEvent(KubeEvent),
    Endpoints(Endpoints),
    ResourceQuota(ResourceQuota),
    LimitRange(LimitRange),
    PersistentVolumeClaim(PersistentVolumeClaim),
    // Core v1 — cluster-scoped
    Namespace(Namespace),
    Node(Node),
    PersistentVolume(PersistentVolume),
    // apps/v1
    Deployment(Deployment),
    StatefulSet(StatefulSet),
    DaemonSet(DaemonSet),
    ReplicaSet(ReplicaSet),
    // batch/v1
    Job(Job),
    CronJob(CronJob),
    // networking.k8s.io/v1
    Ingress(Ingress),
    NetworkPolicy(NetworkPolicy),
    // storage.k8s.io/v1
    StorageClass(StorageClass),
    // rbac.authorization.k8s.io/v1
    Role(Role),
    ClusterRole(ClusterRole),
    RoleBinding(RoleBinding),
    ClusterRoleBinding(ClusterRoleBinding),
}

impl Resource {
    pub fn kind(&self) -> &str {
        match self {
            Resource::Pod(_) => "Pod",
            Resource::Service(_) => "Service",
            Resource::ConfigMap(_) => "ConfigMap",
            Resource::Secret(_) => "Secret",
            Resource::ServiceAccount(_) => "ServiceAccount",
            Resource::KubeEvent(_) => "KubeEvent",
            Resource::Endpoints(_) => "Endpoints",
            Resource::ResourceQuota(_) => "ResourceQuota",
            Resource::LimitRange(_) => "LimitRange",
            Resource::PersistentVolumeClaim(_) => "PersistentVolumeClaim",
            Resource::Namespace(_) => "Namespace",
            Resource::Node(_) => "Node",
            Resource::PersistentVolume(_) => "PersistentVolume",
            Resource::Deployment(_) => "Deployment",
            Resource::StatefulSet(_) => "StatefulSet",
            Resource::DaemonSet(_) => "DaemonSet",
            Resource::ReplicaSet(_) => "ReplicaSet",
            Resource::Job(_) => "Job",
            Resource::CronJob(_) => "CronJob",
            Resource::Ingress(_) => "Ingress",
            Resource::NetworkPolicy(_) => "NetworkPolicy",
            Resource::StorageClass(_) => "StorageClass",
            Resource::Role(_) => "Role",
            Resource::ClusterRole(_) => "ClusterRole",
            Resource::RoleBinding(_) => "RoleBinding",
            Resource::ClusterRoleBinding(_) => "ClusterRoleBinding",
        }
    }

    pub fn metadata(&self) -> &ObjectMeta {
        match self {
            Resource::Pod(r) => &r.metadata,
            Resource::Service(r) => &r.metadata,
            Resource::ConfigMap(r) => &r.metadata,
            Resource::Secret(r) => &r.metadata,
            Resource::ServiceAccount(r) => &r.metadata,
            Resource::KubeEvent(r) => &r.metadata,
            Resource::Endpoints(r) => &r.metadata,
            Resource::ResourceQuota(r) => &r.metadata,
            Resource::LimitRange(r) => &r.metadata,
            Resource::PersistentVolumeClaim(r) => &r.metadata,
            Resource::Namespace(r) => &r.metadata,
            Resource::Node(r) => &r.metadata,
            Resource::PersistentVolume(r) => &r.metadata,
            Resource::Deployment(r) => &r.metadata,
            Resource::StatefulSet(r) => &r.metadata,
            Resource::DaemonSet(r) => &r.metadata,
            Resource::ReplicaSet(r) => &r.metadata,
            Resource::Job(r) => &r.metadata,
            Resource::CronJob(r) => &r.metadata,
            Resource::Ingress(r) => &r.metadata,
            Resource::NetworkPolicy(r) => &r.metadata,
            Resource::StorageClass(r) => &r.metadata,
            Resource::Role(r) => &r.metadata,
            Resource::ClusterRole(r) => &r.metadata,
            Resource::RoleBinding(r) => &r.metadata,
            Resource::ClusterRoleBinding(r) => &r.metadata,
        }
    }

    pub fn name(&self) -> &str { &self.metadata().name }
    pub fn namespace(&self) -> &str { &self.metadata().namespace }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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
    fn test_resource_kind_configmap() {
        let cm = Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            metadata: ObjectMeta::new("test", "default"),
            data: HashMap::new(),
        });
        assert_eq!(cm.kind(), "ConfigMap");
        assert_eq!(cm.name(), "test");
    }

    #[test]
    fn test_resource_kind_all_variants() {
        let meta = || ObjectMeta::new("x", "default");
        let cluster_meta = || ObjectMeta::new("x", "");
        let role_ref = || RoleRef { api_group: "rbac.authorization.k8s.io".into(), kind: "Role".into(), name: "r".into() };

        assert_eq!(Resource::StatefulSet(StatefulSet { api_version: "apps/v1".into(), kind: "StatefulSet".into(), metadata: meta(), spec: StatefulSetSpec::default(), status: StatefulSetStatus::default() }).kind(), "StatefulSet");
        assert_eq!(Resource::DaemonSet(DaemonSet { api_version: "apps/v1".into(), kind: "DaemonSet".into(), metadata: meta(), spec: DaemonSetSpec::default(), status: DaemonSetStatus::default() }).kind(), "DaemonSet");
        assert_eq!(Resource::ReplicaSet(ReplicaSet { api_version: "apps/v1".into(), kind: "ReplicaSet".into(), metadata: meta(), spec: ReplicaSetSpec::default(), status: ReplicaSetStatus::default() }).kind(), "ReplicaSet");
        assert_eq!(Resource::Job(Job { api_version: "batch/v1".into(), kind: "Job".into(), metadata: meta(), spec: JobSpec::default(), status: JobStatus::default() }).kind(), "Job");
        assert_eq!(Resource::CronJob(CronJob { api_version: "batch/v1".into(), kind: "CronJob".into(), metadata: meta(), spec: CronJobSpec::default(), status: CronJobStatus::default() }).kind(), "CronJob");
        assert_eq!(Resource::Ingress(Ingress { api_version: "networking.k8s.io/v1".into(), kind: "Ingress".into(), metadata: meta(), spec: IngressSpec::default(), status: IngressStatus::default() }).kind(), "Ingress");
        assert_eq!(Resource::NetworkPolicy(NetworkPolicy { api_version: "networking.k8s.io/v1".into(), kind: "NetworkPolicy".into(), metadata: meta(), spec: NetworkPolicySpec::default() }).kind(), "NetworkPolicy");
        assert_eq!(Resource::Node(Node { api_version: "v1".into(), kind: "Node".into(), metadata: cluster_meta(), spec: NodeSpec::default(), status: NodeStatus::default() }).kind(), "Node");
        assert_eq!(Resource::Role(Role { api_version: "rbac.authorization.k8s.io/v1".into(), kind: "Role".into(), metadata: meta(), rules: vec![] }).kind(), "Role");
        assert_eq!(Resource::ClusterRole(ClusterRole { api_version: "rbac.authorization.k8s.io/v1".into(), kind: "ClusterRole".into(), metadata: cluster_meta(), rules: vec![], aggregation_rule: None }).kind(), "ClusterRole");
        assert_eq!(Resource::RoleBinding(RoleBinding { api_version: "rbac.authorization.k8s.io/v1".into(), kind: "RoleBinding".into(), metadata: meta(), subjects: vec![], role_ref: role_ref() }).kind(), "RoleBinding");
        assert_eq!(Resource::ClusterRoleBinding(ClusterRoleBinding { api_version: "rbac.authorization.k8s.io/v1".into(), kind: "ClusterRoleBinding".into(), metadata: cluster_meta(), subjects: vec![], role_ref: role_ref() }).kind(), "ClusterRoleBinding");
    }
}

// ── Extended resource field tests ─────────────────────────────────────────────
// upstream: kubernetes/kubernetes pkg/apis/core/types.go

#[cfg(test)]
mod tests_fields {
    use super::*;
    use std::collections::HashMap;

    const TENANT: &str = "tenant-fields";

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectMeta
    #[test]
    fn test_objectmeta_labels() {
        let mut m = ObjectMeta::new("obj", TENANT);
        m.labels.insert("app".into(), "nginx".into());
        m.labels.insert("tenant".into(), TENANT.into());
        assert_eq!(m.labels["app"], "nginx");
        assert_eq!(m.labels["tenant"], TENANT);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectMeta
    #[test]
    fn test_objectmeta_annotations() {
        let mut m = ObjectMeta::new("obj", TENANT);
        m.annotations.insert("deployment.kubernetes.io/revision".into(), "3".into());
        assert_eq!(m.annotations.len(), 1);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectMeta
    #[test]
    fn test_objectmeta_owner_references() {
        let mut m = ObjectMeta::new("rs-abc", TENANT);
        m.owner_references.push(OwnerReference {
            api_version: "apps/v1".into(),
            kind: "Deployment".into(),
            name: "web".into(),
            uid: uuid::Uuid::new_v4(),
        });
        assert_eq!(m.owner_references.len(), 1);
        assert_eq!(m.owner_references[0].kind, "Deployment");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectMeta
    #[test]
    fn test_objectmeta_finalizers() {
        let mut m = ObjectMeta::new("obj", TENANT);
        m.finalizers.push("kubernetes.io/pvc-protection".into());
        m.finalizers.push("custom.io/finalizer".into());
        assert_eq!(m.finalizers.len(), 2);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectMeta
    #[test]
    fn test_objectmeta_deletion_timestamp_none_by_default() {
        let m = ObjectMeta::new("obj", TENANT);
        assert!(m.deletion_timestamp.is_none());
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectMeta
    #[test]
    fn test_objectmeta_resource_version_starts_at_1() {
        let m = ObjectMeta::new("obj", TENANT);
        assert_eq!(m.resource_version, 1);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PodSpec
    #[test]
    fn test_pod_spec_default_restart_policy() {
        let spec = PodSpec::default();
        assert_eq!(spec.restart_policy, "Always");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PodSpec
    #[test]
    fn test_pod_spec_containers() {
        let spec = PodSpec {
            containers: vec![
                ContainerDef {
                    name: "main".into(),
                    image: "nginx:1.25".into(),
                    command: vec!["/bin/sh".into()],
                    args: vec!["-c".into(), "echo hello".into()],
                    env: vec![],
                    ports: vec![ContainerPort { name: Some("http".into()), container_port: 80, protocol: "TCP".into() }],
                    resources: ResourceRequirements::default(),
                    volume_mounts: vec![],
                    liveness_probe: None,
                    readiness_probe: None,
                    image_pull_policy: "IfNotPresent".into(),
                }
            ],
            ..PodSpec::default()
        };
        assert_eq!(spec.containers.len(), 1);
        assert_eq!(spec.containers[0].image, "nginx:1.25");
        assert_eq!(spec.containers[0].ports[0].container_port, 80);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PodSpec
    #[test]
    fn test_pod_spec_init_containers() {
        let spec = PodSpec {
            init_containers: vec![
                ContainerDef {
                    name: "init".into(),
                    image: "busybox".into(),
                    command: vec!["sh".into()],
                    args: vec!["-c".into(), "echo init".into()],
                    env: vec![], ports: vec![],
                    resources: ResourceRequirements::default(),
                    volume_mounts: vec![],
                    liveness_probe: None, readiness_probe: None,
                    image_pull_policy: "Always".into(),
                }
            ],
            ..PodSpec::default()
        };
        assert_eq!(spec.init_containers.len(), 1);
        assert_eq!(spec.init_containers[0].name, "init");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::EnvVar
    #[test]
    fn test_env_var_literal() {
        let env = EnvVar { name: "DATABASE_URL".into(), value: Some("postgres://localhost/db".into()), value_from: None };
        assert_eq!(env.value.as_deref(), Some("postgres://localhost/db"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::EnvVar
    #[test]
    fn test_env_var_from_secret() {
        let env = EnvVar {
            name: "PASSWORD".into(),
            value: None,
            value_from: Some(EnvVarSource {
                config_map_key_ref: None,
                secret_key_ref: Some(KeyRef { name: "db-secret".into(), key: "password".into() }),
            }),
        };
        let vf = env.value_from.unwrap();
        assert_eq!(vf.secret_key_ref.as_ref().unwrap().name, "db-secret");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::EnvVar
    #[test]
    fn test_env_var_from_configmap() {
        let env = EnvVar {
            name: "CONFIG_VAL".into(),
            value: None,
            value_from: Some(EnvVarSource {
                config_map_key_ref: Some(KeyRef { name: "app-config".into(), key: "setting".into() }),
                secret_key_ref: None,
            }),
        };
        let vf = env.value_from.unwrap();
        assert_eq!(vf.config_map_key_ref.as_ref().unwrap().key, "setting");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ResourceRequirements
    #[test]
    fn test_resource_requirements() {
        let mut rr = ResourceRequirements::default();
        rr.requests.insert("cpu".into(), "100m".into());
        rr.requests.insert("memory".into(), "128Mi".into());
        rr.limits.insert("cpu".into(), "500m".into());
        rr.limits.insert("memory".into(), "256Mi".into());
        assert_eq!(rr.requests["cpu"], "100m");
        assert_eq!(rr.limits["memory"], "256Mi");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Volume
    #[test]
    fn test_volume_configmap_source() {
        let vol = Volume {
            name: "config-vol".into(),
            config_map: Some(ConfigMapVolumeSource { name: "app-config".into() }),
            secret: None, empty_dir: None, host_path: None, persistent_volume_claim: None,
        };
        assert_eq!(vol.config_map.as_ref().unwrap().name, "app-config");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Volume
    #[test]
    fn test_volume_secret_source() {
        let vol = Volume {
            name: "secret-vol".into(),
            config_map: None,
            secret: Some(SecretVolumeSource { secret_name: "tls-certs".into() }),
            empty_dir: None, host_path: None, persistent_volume_claim: None,
        };
        assert_eq!(vol.secret.as_ref().unwrap().secret_name, "tls-certs");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Volume
    #[test]
    fn test_volume_empty_dir() {
        let vol = Volume {
            name: "cache".into(),
            config_map: None, secret: None,
            empty_dir: Some(EmptyDirVolumeSource { medium: Some("Memory".into()) }),
            host_path: None, persistent_volume_claim: None,
        };
        assert_eq!(vol.empty_dir.as_ref().unwrap().medium.as_deref(), Some("Memory"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Volume
    #[test]
    fn test_volume_pvc_source() {
        let vol = Volume {
            name: "data".into(),
            config_map: None, secret: None, empty_dir: None, host_path: None,
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource { claim_name: "data-pvc".into(), read_only: false }),
        };
        assert_eq!(vol.persistent_volume_claim.as_ref().unwrap().claim_name, "data-pvc");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Toleration
    #[test]
    fn test_toleration() {
        let tol = Toleration {
            key: Some("node.kubernetes.io/not-ready".into()),
            operator: "Exists".into(),
            value: None,
            effect: Some("NoExecute".into()),
        };
        assert_eq!(tol.operator, "Exists");
        assert_eq!(tol.effect.as_deref(), Some("NoExecute"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/apps/types.go::DeploymentSpec
    #[test]
    fn test_deployment_spec_default_replicas() {
        let spec = DeploymentSpec::default();
        assert_eq!(spec.replicas, 1);
    }

    // upstream: kubernetes/kubernetes pkg/apis/apps/types.go::DeploymentStrategy
    #[test]
    fn test_deployment_strategy_rolling_update() {
        let spec = DeploymentSpec {
            strategy: DeploymentStrategy {
                strategy_type: "RollingUpdate".into(),
                rolling_update: Some(RollingUpdateDeployment {
                    max_unavailable: "25%".into(),
                    max_surge: "25%".into(),
                }),
            },
            ..DeploymentSpec::default()
        };
        assert_eq!(spec.strategy.strategy_type, "RollingUpdate");
        let ru = spec.strategy.rolling_update.unwrap();
        assert_eq!(ru.max_unavailable, "25%");
        assert_eq!(ru.max_surge, "25%");
    }

    // upstream: kubernetes/kubernetes pkg/apis/apps/types.go::DeploymentStrategy
    #[test]
    fn test_deployment_strategy_recreate() {
        let spec = DeploymentSpec {
            strategy: DeploymentStrategy {
                strategy_type: "Recreate".into(),
                rolling_update: None,
            },
            ..DeploymentSpec::default()
        };
        assert_eq!(spec.strategy.strategy_type, "Recreate");
        assert!(spec.strategy.rolling_update.is_none());
    }

    // upstream: kubernetes/kubernetes pkg/apis/apps/types.go::DeploymentStatus
    #[test]
    fn test_deployment_status_defaults() {
        let status = DeploymentStatus::default();
        assert_eq!(status.replicas, 0);
        assert_eq!(status.ready_replicas, 0);
        assert!(status.conditions.is_empty());
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ServiceSpec
    #[test]
    fn test_service_spec_clusterip() {
        let spec = ServiceSpec {
            service_type: "ClusterIP".into(),
            selector: HashMap::from([("app".into(), "web".into())]),
            ports: vec![ServicePort { name: Some("http".into()), port: 80, target_port: 8080, protocol: "TCP".into() }],
            cluster_ip: Some("10.96.100.1".into()),
        };
        assert_eq!(spec.service_type, "ClusterIP");
        assert_eq!(spec.ports[0].port, 80);
        assert_eq!(spec.cluster_ip.as_deref(), Some("10.96.100.1"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ServiceSpec
    #[test]
    fn test_service_spec_nodeport() {
        let spec = ServiceSpec {
            service_type: "NodePort".into(),
            selector: HashMap::new(),
            ports: vec![ServicePort { name: None, port: 80, target_port: 8080, protocol: "TCP".into() }],
            cluster_ip: None,
        };
        assert_eq!(spec.service_type, "NodePort");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ServiceSpec
    #[test]
    fn test_service_multi_port() {
        let spec = ServiceSpec {
            service_type: "ClusterIP".into(),
            selector: HashMap::new(),
            ports: vec![
                ServicePort { name: Some("http".into()), port: 80, target_port: 8080, protocol: "TCP".into() },
                ServicePort { name: Some("https".into()), port: 443, target_port: 8443, protocol: "TCP".into() },
            ],
            cluster_ip: None,
        };
        assert_eq!(spec.ports.len(), 2);
        assert_eq!(spec.ports[1].port, 443);
    }

    // upstream: kubernetes/kubernetes pkg/apis/networking/types.go::NetworkPolicySpec
    #[test]
    fn test_networkpolicy_ingress_rule() {
        let spec = NetworkPolicySpec {
            pod_selector: LabelSelector::default(),
            ingress: vec![NetworkPolicyIngressRule {
                from: vec![NetworkPolicyPeer {
                    pod_selector: Some(LabelSelector {
                        match_labels: HashMap::from([("app".into(), "frontend".into())]),
                        match_expressions: vec![],
                    }),
                    namespace_selector: None,
                    ip_block: None,
                }],
                ports: vec![NetworkPolicyPort { port: Some(8080), protocol: Some("TCP".into()) }],
            }],
            egress: vec![],
            policy_types: vec!["Ingress".into()],
        };
        assert_eq!(spec.ingress.len(), 1);
        assert_eq!(spec.ingress[0].ports[0].port, Some(8080));
    }

    // upstream: kubernetes/kubernetes pkg/apis/networking/types.go::NetworkPolicySpec
    #[test]
    fn test_networkpolicy_egress_rule() {
        let spec = NetworkPolicySpec {
            pod_selector: LabelSelector::default(),
            ingress: vec![],
            egress: vec![NetworkPolicyEgressRule {
                to: vec![],
                ports: vec![NetworkPolicyPort { port: Some(5432), protocol: Some("TCP".into()) }],
            }],
            policy_types: vec!["Egress".into()],
        };
        assert_eq!(spec.egress.len(), 1);
        assert_eq!(spec.egress[0].ports[0].port, Some(5432));
    }

    // upstream: kubernetes/kubernetes pkg/apis/networking/types.go::NetworkPolicySpec
    #[test]
    fn test_networkpolicy_ipblock() {
        let spec = NetworkPolicySpec {
            pod_selector: LabelSelector::default(),
            ingress: vec![NetworkPolicyIngressRule {
                from: vec![NetworkPolicyPeer {
                    pod_selector: None,
                    namespace_selector: None,
                    ip_block: Some(IPBlock { cidr: "192.168.0.0/16".into(), except: vec!["192.168.1.0/24".into()] }),
                }],
                ports: vec![],
            }],
            egress: vec![],
            policy_types: vec!["Ingress".into()],
        };
        let block = spec.ingress[0].from[0].ip_block.as_ref().unwrap();
        assert_eq!(block.cidr, "192.168.0.0/16");
        assert_eq!(block.except.len(), 1);
    }

    // upstream: kubernetes/kubernetes pkg/apis/rbac/types.go::PolicyRule
    #[test]
    fn test_policy_rule_pods_get_list() {
        let rule = PolicyRule {
            api_groups: vec!["".into()],
            resources: vec!["pods".into()],
            verbs: vec!["get".into(), "list".into(), "watch".into()],
            resource_names: vec![],
        };
        assert!(rule.verbs.contains(&"get".to_string()));
        assert!(rule.verbs.contains(&"list".to_string()));
        assert_eq!(rule.api_groups[0], "");
    }

    // upstream: kubernetes/kubernetes pkg/apis/rbac/types.go::PolicyRule
    #[test]
    fn test_policy_rule_all_verbs() {
        let rule = PolicyRule {
            api_groups: vec!["apps".into()],
            resources: vec!["deployments".into()],
            verbs: vec!["get".into(), "list".into(), "watch".into(), "create".into(), "update".into(), "patch".into(), "delete".into()],
            resource_names: vec![],
        };
        assert_eq!(rule.verbs.len(), 7);
        assert_eq!(rule.api_groups[0], "apps");
    }

    // upstream: kubernetes/kubernetes pkg/apis/rbac/types.go::Subject
    #[test]
    fn test_subject_user() {
        let s = Subject { kind: "User".into(), name: "alice".into(), namespace: None, api_group: Some("rbac.authorization.k8s.io".into()) };
        assert_eq!(s.kind, "User");
        assert_eq!(s.name, "alice");
    }

    // upstream: kubernetes/kubernetes pkg/apis/rbac/types.go::Subject
    #[test]
    fn test_subject_service_account() {
        let s = Subject { kind: "ServiceAccount".into(), name: "default".into(), namespace: Some(TENANT.into()), api_group: None };
        assert_eq!(s.kind, "ServiceAccount");
        assert_eq!(s.namespace.as_deref(), Some(TENANT));
    }

    // upstream: kubernetes/kubernetes pkg/apis/rbac/types.go::AggregationRule
    #[test]
    fn test_aggregation_rule() {
        let ar = AggregationRule {
            cluster_role_selectors: vec![
                LabelSelector { match_labels: HashMap::from([("rbac.authorization.k8s.io/aggregate-to-admin".into(), "true".into())]), match_expressions: vec![] },
            ],
        };
        assert_eq!(ar.cluster_role_selectors.len(), 1);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PersistentVolumeSpec
    #[test]
    fn test_pv_spec_capacity() {
        let mut spec = PersistentVolumeSpec::default();
        spec.capacity.insert("storage".into(), "10Gi".into());
        spec.storage_class_name = Some("standard".into());
        assert_eq!(spec.capacity["storage"], "10Gi");
        assert_eq!(spec.reclaim_policy, "Retain");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PersistentVolumeClaimSpec
    #[test]
    fn test_pvc_spec_access_modes() {
        let spec = PersistentVolumeClaimSpec {
            access_modes: vec!["ReadWriteMany".into()],
            resources: ResourceRequirements { requests: HashMap::from([("storage".into(), "5Gi".into())]), limits: HashMap::new() },
            storage_class_name: Some("fast".into()),
            volume_name: None,
            volume_mode: Some("Filesystem".into()),
        };
        assert_eq!(spec.access_modes[0], "ReadWriteMany");
        assert_eq!(spec.resources.requests["storage"], "5Gi");
        assert_eq!(spec.storage_class_name.as_deref(), Some("fast"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::NodeSpec
    #[test]
    fn test_node_spec_taint() {
        let spec = NodeSpec {
            pod_cidr: Some("10.0.1.0/24".into()),
            provider_id: Some("aws:///us-east-1a/i-abc123".into()),
            unschedulable: false,
            taints: vec![Taint { key: "node-role.kubernetes.io/master".into(), value: None, effect: "NoSchedule".into() }],
        };
        assert_eq!(spec.taints.len(), 1);
        assert_eq!(spec.taints[0].effect, "NoSchedule");
        assert_eq!(spec.pod_cidr.as_deref(), Some("10.0.1.0/24"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::NodeStatus
    #[test]
    fn test_node_status_conditions() {
        let mut status = NodeStatus::default();
        status.conditions.push(NodeCondition {
            condition_type: "Ready".into(),
            status: "True".into(),
            last_transition_time: chrono::Utc::now(),
            reason: Some("KubeletReady".into()),
            message: Some("kubelet is posting ready status".into()),
        });
        status.capacity.insert("cpu".into(), "4".into());
        status.capacity.insert("memory".into(), "16Gi".into());
        assert_eq!(status.conditions.len(), 1);
        assert_eq!(status.conditions[0].condition_type, "Ready");
        assert_eq!(status.capacity["cpu"], "4");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::LimitRangeItem
    #[test]
    fn test_limit_range_item() {
        let item = LimitRangeItem {
            limit_type: "Container".into(),
            max: HashMap::from([("cpu".into(), "2".into()), ("memory".into(), "1Gi".into())]),
            min: HashMap::from([("cpu".into(), "100m".into()), ("memory".into(), "64Mi".into())]),
            default: HashMap::from([("cpu".into(), "500m".into())]),
            default_request: HashMap::from([("cpu".into(), "100m".into())]),
        };
        assert_eq!(item.limit_type, "Container");
        assert_eq!(item.max["cpu"], "2");
        assert_eq!(item.min["memory"], "64Mi");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ResourceQuotaSpec
    #[test]
    fn test_resource_quota_spec() {
        let mut spec = ResourceQuotaSpec::default();
        spec.hard.insert("pods".into(), "10".into());
        spec.hard.insert("services".into(), "5".into());
        spec.hard.insert("requests.cpu".into(), "4".into());
        spec.hard.insert("requests.memory".into(), "8Gi".into());
        assert_eq!(spec.hard.len(), 4);
        assert_eq!(spec.hard["pods"], "10");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::EndpointSubset
    #[test]
    fn test_endpoints_with_subsets() {
        let ep = Endpoints {
            api_version: "v1".into(), kind: "Endpoints".into(),
            metadata: ObjectMeta::new("my-svc", TENANT),
            subsets: vec![EndpointSubset {
                addresses: vec![
                    EndpointAddress { ip: "10.0.1.5".into(), hostname: Some("pod-a".into()), target_ref: None },
                    EndpointAddress { ip: "10.0.1.6".into(), hostname: Some("pod-b".into()), target_ref: None },
                ],
                not_ready_addresses: vec![],
                ports: vec![EndpointPort { name: Some("http".into()), port: 8080, protocol: "TCP".into() }],
            }],
        };
        assert_eq!(ep.subsets[0].addresses.len(), 2);
        assert_eq!(ep.subsets[0].ports[0].port, 8080);
    }

    // upstream: kubernetes/kubernetes pkg/apis/batch/types.go::JobSpec
    #[test]
    fn test_job_spec_completions_parallelism() {
        let spec = JobSpec {
            completions: Some(5),
            parallelism: Some(2),
            backoff_limit: Some(3),
            active_deadline_seconds: Some(3600),
            template: PodTemplateSpec::default(),
        };
        assert_eq!(spec.completions, Some(5));
        assert_eq!(spec.parallelism, Some(2));
        assert_eq!(spec.active_deadline_seconds, Some(3600));
    }

    // upstream: kubernetes/kubernetes pkg/apis/batch/types.go::CronJobSpec
    #[test]
    fn test_cronjob_spec_schedule() {
        let spec = CronJobSpec {
            schedule: "*/5 * * * *".into(),
            concurrency_policy: "Forbid".into(),
            suspend: true,
            successful_jobs_history_limit: Some(5),
            failed_jobs_history_limit: Some(3),
            job_template: JobTemplateSpec::default(),
        };
        assert_eq!(spec.schedule, "*/5 * * * *");
        assert_eq!(spec.concurrency_policy, "Forbid");
        assert!(spec.suspend);
    }

    // upstream: kubernetes/kubernetes pkg/apis/networking/types.go::IngressSpec
    #[test]
    fn test_ingress_spec_rules() {
        let spec = IngressSpec {
            ingress_class_name: Some("nginx".into()),
            rules: vec![IngressRule {
                host: Some("example.com".into()),
                http: Some(HTTPIngressRuleValue {
                    paths: vec![HTTPIngressPath {
                        path: "/".into(),
                        path_type: "Prefix".into(),
                        backend: IngressBackend {
                            service: IngressServiceBackend {
                                name: "web".into(),
                                port: ServiceBackendPort { number: 80, name: None },
                            },
                        },
                    }],
                }),
            }],
            tls: vec![IngressTLS { hosts: vec!["example.com".into()], secret_name: Some("tls-cert".into()) }],
            default_backend: None,
        };
        assert_eq!(spec.rules[0].host.as_deref(), Some("example.com"));
        assert_eq!(spec.rules[0].http.as_ref().unwrap().paths[0].path_type, "Prefix");
        assert_eq!(spec.tls[0].secret_name.as_deref(), Some("tls-cert"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Probe
    #[test]
    fn test_liveness_probe_http() {
        let probe = Probe {
            http_get: Some(HttpGetAction { path: "/healthz".into(), port: 8080 }),
            exec: None, tcp_socket: None,
            initial_delay_seconds: 10,
            period_seconds: 30,
            timeout_seconds: 5,
            failure_threshold: 3,
        };
        assert_eq!(probe.http_get.as_ref().unwrap().path, "/healthz");
        assert_eq!(probe.initial_delay_seconds, 10);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Probe
    #[test]
    fn test_readiness_probe_exec() {
        let probe = Probe {
            http_get: None,
            exec: Some(ExecAction { command: vec!["cat".into(), "/tmp/ready".into()] }),
            tcp_socket: None,
            initial_delay_seconds: 5,
            period_seconds: 10,
            timeout_seconds: 3,
            failure_threshold: 3,
        };
        let cmd = probe.exec.unwrap().command;
        assert_eq!(cmd[0], "cat");
        assert_eq!(cmd[1], "/tmp/ready");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::Probe
    #[test]
    fn test_liveness_probe_tcp_socket() {
        let probe = Probe {
            http_get: None, exec: None,
            tcp_socket: Some(TcpSocketAction { port: 5432 }),
            initial_delay_seconds: 15,
            period_seconds: 20,
            timeout_seconds: 5,
            failure_threshold: 6,
        };
        assert_eq!(probe.tcp_socket.unwrap().port, 5432);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::LabelSelector
    #[test]
    fn test_label_selector_match_labels() {
        let sel = LabelSelector {
            match_labels: HashMap::from([("app".into(), "web".into()), ("tier".into(), "frontend".into())]),
            match_expressions: vec![],
        };
        assert_eq!(sel.match_labels.len(), 2);
        assert_eq!(sel.match_labels["tier"], "frontend");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::LabelSelector
    #[test]
    fn test_label_selector_match_expressions_in() {
        let sel = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "env".into(),
                operator: "In".into(),
                values: vec!["production".into(), "staging".into()],
            }],
        };
        assert_eq!(sel.match_expressions[0].operator, "In");
        assert_eq!(sel.match_expressions[0].values.len(), 2);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::LabelSelector
    #[test]
    fn test_label_selector_match_expressions_not_in() {
        let sel = LabelSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![LabelSelectorRequirement {
                key: "tier".into(),
                operator: "NotIn".into(),
                values: vec!["backend".into()],
            }],
        };
        assert_eq!(sel.match_expressions[0].operator, "NotIn");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectReference
    #[test]
    fn test_object_reference_default() {
        let r = ObjectReference::default();
        assert!(r.kind.is_empty());
        assert!(r.name.is_empty());
        assert!(r.uid.is_none());
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ObjectReference
    #[test]
    fn test_object_reference_fields() {
        let r = ObjectReference {
            kind: "Pod".into(),
            name: "my-pod".into(),
            namespace: TENANT.into(),
            api_version: Some("v1".into()),
            uid: Some(uuid::Uuid::new_v4()),
        };
        assert_eq!(r.kind, "Pod");
        assert_eq!(r.namespace, TENANT);
        assert!(r.uid.is_some());
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PodCondition
    #[test]
    fn test_pod_condition() {
        let cond = PodCondition {
            condition_type: "Ready".into(),
            status: "True".into(),
            last_transition_time: chrono::Utc::now(),
            reason: Some("PodReady".into()),
            message: Some("all containers running".into()),
        };
        assert_eq!(cond.condition_type, "Ready");
        assert_eq!(cond.status, "True");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PodStatus
    #[test]
    fn test_pod_status_fields() {
        let mut status = PodStatus::default();
        status.phase = "Running".into();
        status.pod_ip = Some("10.0.1.5".into());
        status.host_ip = Some("192.168.1.1".into());
        assert_eq!(status.phase, "Running");
        assert_eq!(status.pod_ip.as_deref(), Some("10.0.1.5"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ContainerStatus
    #[test]
    fn test_container_status() {
        let cs = ContainerStatus {
            name: "web".into(),
            ready: true,
            restart_count: 0,
            image: "nginx:1.25".into(),
            started: true,
        };
        assert!(cs.ready);
        assert_eq!(cs.restart_count, 0);
        assert_eq!(cs.image, "nginx:1.25");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::NamespaceStatus
    #[test]
    fn test_namespace_status_active() {
        let status = NamespaceStatus { phase: "Active".into() };
        assert_eq!(status.phase, "Active");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::NamespaceStatus
    #[test]
    fn test_namespace_status_terminating() {
        let status = NamespaceStatus { phase: "Terminating".into() };
        assert_eq!(status.phase, "Terminating");
    }

    // upstream: kubernetes/kubernetes pkg/apis/apps/types.go::StatefulSetSpec
    #[test]
    fn test_statefulset_spec_service_name() {
        let spec = StatefulSetSpec {
            replicas: 3,
            service_name: "my-svc".into(),
            ..StatefulSetSpec::default()
        };
        assert_eq!(spec.replicas, 3);
        assert_eq!(spec.service_name, "my-svc");
    }

    // upstream: kubernetes/kubernetes pkg/apis/apps/types.go::DaemonSetUpdateStrategy
    #[test]
    fn test_daemonset_update_strategy() {
        let spec = DaemonSetSpec {
            update_strategy: DaemonSetUpdateStrategy { update_strategy_type: "OnDelete".into() },
            ..DaemonSetSpec::default()
        };
        assert_eq!(spec.update_strategy.update_strategy_type, "OnDelete");
    }

    // upstream: kubernetes/kubernetes pkg/apis/apps/types.go::ReplicaSetStatus
    #[test]
    fn test_replicaset_status() {
        let mut status = ReplicaSetStatus::default();
        status.replicas = 3;
        status.ready_replicas = 2;
        status.available_replicas = 2;
        assert_eq!(status.replicas, 3);
        assert_eq!(status.ready_replicas, 2);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::KubeEvent
    #[test]
    fn test_event_source() {
        let src = EventSource { component: "kubelet".into(), host: Some("node1".into()) };
        assert_eq!(src.component, "kubelet");
        assert_eq!(src.host.as_deref(), Some("node1"));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::ServiceAccount
    #[test]
    fn test_serviceaccount_image_pull_secrets() {
        let sa = ServiceAccount {
            api_version: "v1".into(), kind: "ServiceAccount".into(),
            metadata: ObjectMeta::new("default", TENANT),
            secrets: vec![],
            image_pull_secrets: vec![LocalObjectReference { name: "registry-creds".into() }],
            automount_service_account_token: Some(true),
        };
        assert_eq!(sa.image_pull_secrets.len(), 1);
        assert_eq!(sa.image_pull_secrets[0].name, "registry-creds");
        assert_eq!(sa.automount_service_account_token, Some(true));
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::StorageClass
    #[test]
    fn test_storageclass_fields() {
        let sc = StorageClass {
            api_version: "storage.k8s.io/v1".into(),
            kind: "StorageClass".into(),
            metadata: ObjectMeta::new("premium", ""),
            provisioner: "ebs.csi.aws.com".into(),
            parameters: HashMap::from([("type".into(), "gp3".into()), ("iopsPerGB".into(), "50".into())]),
            reclaim_policy: Some("Delete".into()),
            volume_binding_mode: Some("WaitForFirstConsumer".into()),
            allow_volume_expansion: true,
        };
        assert_eq!(sc.provisioner, "ebs.csi.aws.com");
        assert_eq!(sc.parameters["type"], "gp3");
        assert!(sc.allow_volume_expansion);
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PersistentVolumeStatus
    #[test]
    fn test_pv_status_phases() {
        let avail = PersistentVolumeStatus::default();
        assert_eq!(avail.phase, "Available");

        let bound = PersistentVolumeStatus { phase: "Bound".into(), reason: None, message: None };
        assert_eq!(bound.phase, "Bound");

        let released = PersistentVolumeStatus { phase: "Released".into(), reason: Some("CrashLoopEvict".into()), message: None };
        assert_eq!(released.phase, "Released");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::PersistentVolumeClaimStatus
    #[test]
    fn test_pvc_status_phases() {
        let pending = PersistentVolumeClaimStatus::default();
        assert_eq!(pending.phase, "Pending");

        let bound = PersistentVolumeClaimStatus { phase: "Bound".into(), access_modes: vec!["ReadWriteOnce".into()], capacity: HashMap::new() };
        assert_eq!(bound.phase, "Bound");
    }

    // upstream: kubernetes/kubernetes pkg/apis/core/types.go::NodeSystemInfo
    #[test]
    fn test_node_system_info() {
        let mut info = NodeSystemInfo::default();
        info.kernel_version = "5.15.0".into();
        info.architecture = "amd64".into();
        info.operating_system = "linux".into();
        info.container_runtime_version = "containerd://1.6.20".into();
        info.kubelet_version = "v1.29.0".into();
        assert_eq!(info.architecture, "amd64");
        assert_eq!(info.operating_system, "linux");
    }
}

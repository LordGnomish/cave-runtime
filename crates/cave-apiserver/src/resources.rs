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

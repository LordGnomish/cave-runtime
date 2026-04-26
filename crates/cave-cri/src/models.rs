//! Data models for the container runtime.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Specification for creating a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub mounts: Vec<Mount>,
    pub resources: ResourceLimits,
    pub labels: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub hostname: Option<String>,
    pub network_mode: NetworkMode,
    pub restart_policy: RestartPolicy,
}

/// Resource limits enforced via cgroup v2.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub cpu_shares: Option<u64>,
    pub cpu_quota: Option<i64>,
    pub memory_limit: Option<u64>,
    pub pids_limit: Option<u64>,
}

/// A running or stopped container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    pub id: Uuid,
    pub spec: ContainerSpec,
    pub status: ContainerStatus,
    pub pid: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub rootfs_path: PathBuf,
    pub log_path: PathBuf,
    pub health: Option<HealthStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerStatus {
    Created,
    Running,
    Paused,
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub enum NetworkMode {
    Host,
    #[default]
    Bridge,
    None,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub enum RestartPolicy {
    #[default]
    Never,
    OnFailure { max_retries: u32 },
    Always,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mount {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub read_only: bool,
    pub mount_type: MountType,
    /// CRI-side propagation mode (private / rslave / rshared). Mirrors
    /// containerd `pkg/cri/server/container_create_linux.go` mount propagation
    /// translation in v2.2.3.
    #[serde(default)]
    pub propagation: MountPropagation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MountType {
    Bind,
    Volume,
    Tmpfs,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountPropagation {
    #[default]
    Private,
    HostToContainer,
    Bidirectional,
}

/// Cite: containerd `pkg/cri/server/container_create_linux.go`
/// (setOCISecurityContext) v2.2.3 + runc `libcontainer/specconv/spec_linux.go`
/// v1.4.2. Full per-container security knobs that get folded into the OCI
/// runtime spec.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityContext {
    pub run_as_user: Option<u32>,
    pub run_as_group: Option<u32>,
    #[serde(default)]
    pub supplemental_groups: Vec<u32>,
    pub run_as_non_root: bool,
    pub readonly_rootfs: bool,
    pub allow_privilege_escalation: bool,
    pub privileged: bool,
    #[serde(default)]
    pub capabilities_add: Vec<String>,
    #[serde(default)]
    pub capabilities_drop: Vec<String>,
    pub seccomp_profile: Option<SeccompProfile>,
    pub apparmor_profile: Option<String>,
    pub selinux_label: Option<SelinuxLabel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeccompProfile {
    /// `RuntimeDefault` — apply the runtime's built-in seccomp filter.
    RuntimeDefault,
    /// `Unconfined` — disable seccomp entirely.
    Unconfined,
    /// `Localhost` — load the JSON profile at the given path.
    Localhost(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelinuxLabel {
    pub user: Option<String>,
    pub role: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub level: Option<String>,
}

/// OCI image metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciImage {
    pub reference: String,
    pub digest: String,
    pub layers: Vec<OciLayer>,
    pub config: ImageConfig,
    pub size_bytes: u64,
    pub pulled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciLayer {
    pub digest: String,
    pub size: u64,
    pub media_type: String,
    pub local_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageConfig {
    pub entrypoint: Vec<String>,
    pub cmd: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub exposed_ports: Vec<String>,
    pub labels: HashMap<String, String>,
}

/// OCI manifest (Docker Registry HTTP API v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

/// cgroup v2 resource usage stats.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CgroupStats {
    pub cpu_usage_usec: u64,
    pub memory_current: u64,
    pub memory_peak: u64,
    pub pids_current: u64,
}

/// Container exec request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub command: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub tty: bool,
}

/// Parsed image reference: registry/repository:tag@digest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageReference {
    pub registry: String,
    pub repository: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

impl ImageReference {
    pub fn parse(reference: &str) -> Self {
        let (ref_without_digest, digest) = if let Some(idx) = reference.find('@') {
            (&reference[..idx], Some(reference[idx + 1..].to_string()))
        } else {
            (reference, None)
        };

        let (ref_without_tag, tag) = if let Some(idx) = ref_without_digest.rfind(':') {
            if ref_without_digest[idx + 1..].contains('/') {
                (ref_without_digest, None)
            } else {
                (&ref_without_digest[..idx], Some(ref_without_digest[idx + 1..].to_string()))
            }
        } else {
            (ref_without_digest, None)
        };

        let (registry, repository) = if ref_without_tag.contains('/') {
            let first_slash = ref_without_tag.find('/').unwrap();
            let maybe_registry = &ref_without_tag[..first_slash];
            if maybe_registry.contains('.') || maybe_registry.contains(':') || maybe_registry == "localhost" {
                (maybe_registry.to_string(), ref_without_tag[first_slash + 1..].to_string())
            } else {
                ("docker.io".to_string(), ref_without_tag.to_string())
            }
        } else {
            ("docker.io".to_string(), format!("library/{}", ref_without_tag))
        };

        Self { registry, repository, tag, digest }
    }

    pub fn full_reference(&self) -> String {
        let mut s = format!("{}/{}", self.registry, self.repository);
        if let Some(ref tag) = self.tag {
            s.push_str(&format!(":{}", tag));
        }
        if let Some(ref digest) = self.digest {
            s.push_str(&format!("@{}", digest));
        }
        s
    }
}

/// Pod sandbox specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSpec {
    pub name: String,
    pub namespace: String,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub hostname: Option<String>,
    pub dns_config: Option<DnsConfig>,
    pub port_mappings: Vec<PortMapping>,
    pub log_directory: Option<String>,
    pub cgroup_parent: Option<String>,
    /// Runtime handler name from `PodSandboxConfig.runtime_handler`
    /// (Kubernetes `RuntimeClass.handler`). Empty/None → use the registry
    /// default. See KEP-585.
    #[serde(default)]
    pub runtime_handler: Option<String>,
    /// User-namespace mode for the pod (KEP-127). `Host` (default) skips
    /// remapping; `Pod` remaps container UID/GID 0 to a per-pod host
    /// range allocated by `UserNsAllocator`.
    #[serde(default)]
    pub user_namespace_mode: UserNamespaceMode,
}

/// `pod.spec.hostUsers` translation. KEP-127.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserNamespaceMode {
    /// `hostUsers: true` — share the host user namespace.
    #[default]
    Host,
    /// `hostUsers: false` — allocate a private user namespace per pod.
    Pod,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DnsConfig {
    pub servers: Vec<String>,
    pub searches: Vec<String>,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub protocol: String,
    pub container_port: u16,
    pub host_port: u16,
    pub host_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxState {
    Ready,
    NotReady,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sandbox {
    pub id: Uuid,
    pub spec: SandboxSpec,
    pub state: SandboxState,
    pub created_at: DateTime<Utc>,
    pub network_ip: Option<String>,
}

/// Process running inside a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerProcess {
    pub pid: u32,
    pub user: String,
    pub command: String,
    pub cpu_percent: f64,
    pub memory_bytes: u64,
}

/// Container checkpoint metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInfo {
    pub container_id: Uuid,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
}

/// A single log line from a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerLogEntry {
    pub timestamp: DateTime<Utc>,
    pub stream: String,
    pub message: String,
}

/// Container resource usage snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStats {
    pub container_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub cgroup: CgroupStats,
    pub cpu_percent: f64,
    pub memory_percent: f64,
}

/// Snapshot kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotKind {
    Committed,
    Active,
    View,
}

/// OCI snapshot (overlayfs layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: Uuid,
    pub name: String,
    pub parent: Option<String>,
    pub labels: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub kind: SnapshotKind,
}

/// Disk usage for a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotUsage {
    pub snapshot_id: Uuid,
    pub size_bytes: u64,
    pub inodes: u64,
}

/// Mount point for a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMount {
    pub kind: String,
    pub source: String,
    pub options: Vec<String>,
}

/// Network attachment status for a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub container_id: Uuid,
    pub network_name: String,
    pub ip_address: Option<String>,
    pub mac_address: Option<String>,
    pub gateway: Option<String>,
    pub interface: Option<String>,
    pub attached: bool,
}

/// Runtime version info (mirrors containerd Version RPC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeVersion {
    pub version: String,
    pub api_version: String,
    pub runtime_name: String,
    pub runtime_version: String,
    pub runtime_api_version: String,
}

/// Single readiness condition in RuntimeStatus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCondition {
    pub kind: String,
    pub status: bool,
    pub reason: String,
    pub message: String,
}

/// Runtime readiness (mirrors containerd Status RPC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub conditions: Vec<RuntimeCondition>,
    /// Runtime handlers advertised to the kubelet (KEP-585). Empty when
    /// no handlers are registered.
    #[serde(default)]
    pub runtime_handlers: Vec<crate::runtime_handler::RuntimeHandler>,
}

/// Node-wide CPU stats.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuStats {
    pub usage_total_usec: u64,
    pub usage_percent: f64,
}

/// Node-wide memory stats.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub usage_percent: f64,
}

/// Aggregate node resource stats across all containers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStats {
    pub timestamp: DateTime<Utc>,
    pub cpu: CpuStats,
    pub memory: MemoryStats,
    pub container_count: usize,
}

/// Sandbox resource stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxStats {
    pub sandbox_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub cgroup: CgroupStats,
    pub container_count: usize,
}

/// Runtime event (create, start, stop, delete, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub id: String,
    pub kind: String,
    pub object_type: String,
    pub object_id: String,
    pub timestamp: DateTime<Utc>,
    pub attributes: HashMap<String, String>,
}

/// Partial update applied to a running container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerUpdate {
    pub resources: Option<ResourceLimits>,
    pub labels: Option<HashMap<String, String>>,
}

/// Result of exec-in-container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

/// Request to tag an image with a new reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageTagRequest {
    pub target: String,
}

/// One layer in an image's build history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageHistoryEntry {
    pub digest: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub size_bytes: u64,
    pub comment: String,
}

/// Health check configuration (exec / http / tcp).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub kind: HealthCheckKind,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub retries: u32,
    pub start_period_secs: u64,
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self {
            kind: HealthCheckKind::Exec { command: vec![] },
            interval_secs: 30,
            timeout_secs: 30,
            retries: 3,
            start_period_secs: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HealthCheckKind {
    Exec { command: Vec<String> },
    Http { url: String, expected_status: u16 },
    Tcp  { host: String, port: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthState {
    Starting,
    Healthy,
    Unhealthy,
}

/// Current health check status for a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub state: HealthState,
    pub failing_streak: u32,
    pub last_output: String,
    pub last_checked_at: Option<DateTime<Utc>>,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self {
            state: HealthState::Starting,
            failing_streak: 0,
            last_output: String::new(),
            last_checked_at: None,
        }
    }
}

/// Log rotation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub max_size_bytes: u64,
    pub max_files: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self { max_size_bytes: 10 * 1024 * 1024, max_files: 5 }
    }
}

/// Extended cgroup v2 stats including io and throttling.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CgroupStatsV2 {
    pub cpu_usage_usec: u64,
    pub cpu_user_usec: u64,
    pub cpu_system_usec: u64,
    pub cpu_nr_throttled: u64,
    pub memory_current: u64,
    pub memory_peak: u64,
    pub memory_swap_current: u64,
    pub pids_current: u64,
    pub pids_max_reached: u64,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_image() {
        let r = ImageReference::parse("nginx");
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.tag, None);
    }

    #[test]
    fn test_parse_tagged_image() {
        let r = ImageReference::parse("nginx:1.25");
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.tag, Some("1.25".into()));
    }

    #[test]
    fn test_parse_registry_image() {
        let r = ImageReference::parse("harbor.cave/library/app:v1");
        assert_eq!(r.registry, "harbor.cave");
        assert_eq!(r.repository, "library/app");
        assert_eq!(r.tag, Some("v1".into()));
    }

    #[test]
    fn test_parse_digest_image() {
        let r = ImageReference::parse("nginx@sha256:abc123");
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.digest, Some("sha256:abc123".into()));
    }

    #[test]
    fn test_parse_full_reference() {
        let r = ImageReference::parse("ghcr.io/org/app:v2.0@sha256:def456");
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "org/app");
        assert_eq!(r.tag, Some("v2.0".into()));
        assert_eq!(r.digest, Some("sha256:def456".into()));
    }

    #[test]
    fn test_container_status_serialization() {
        let s = ContainerStatus::Failed("oom killed".into());
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("oom killed"));
    }

    #[test]
    fn test_resource_limits_default() {
        let r = ResourceLimits::default();
        assert!(r.cpu_shares.is_none());
        assert!(r.memory_limit.is_none());
    }

    // --- ImageReference edge cases ---

    #[test]
    fn test_parse_org_repo_no_registry() {
        let r = ImageReference::parse("myorg/myapp");
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.repository, "myorg/myapp");
        assert_eq!(r.tag, None);
        assert_eq!(r.digest, None);
    }

    #[test]
    fn test_parse_localhost_registry() {
        let r = ImageReference::parse("localhost/myapp:v1");
        assert_eq!(r.registry, "localhost");
        assert_eq!(r.repository, "myapp");
        assert_eq!(r.tag, Some("v1".into()));
    }

    #[test]
    fn test_parse_port_registry() {
        let r = ImageReference::parse("myregistry:5000/myapp:latest");
        assert_eq!(r.registry, "myregistry:5000");
        assert_eq!(r.tag, Some("latest".into()));
    }

    #[test]
    fn test_parse_digest_no_tag() {
        let r = ImageReference::parse("nginx@sha256:deadbeef");
        assert_eq!(r.tag, None);
        assert_eq!(r.digest, Some("sha256:deadbeef".into()));
        assert_eq!(r.registry, "docker.io");
    }

    #[test]
    fn test_parse_bare_name_no_tag() {
        let r = ImageReference::parse("ubuntu");
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.repository, "library/ubuntu");
        assert_eq!(r.tag, None);
    }

    // --- full_reference ---

    #[test]
    fn test_full_reference_no_tag_no_digest() {
        let r = ImageReference { registry: "docker.io".into(), repository: "library/nginx".into(), tag: None, digest: None };
        assert_eq!(r.full_reference(), "docker.io/library/nginx");
    }

    #[test]
    fn test_full_reference_tag_only() {
        let r = ImageReference { registry: "docker.io".into(), repository: "library/nginx".into(), tag: Some("1.25".into()), digest: None };
        assert_eq!(r.full_reference(), "docker.io/library/nginx:1.25");
    }

    #[test]
    fn test_full_reference_digest_only() {
        let r = ImageReference { registry: "docker.io".into(), repository: "library/nginx".into(), tag: None, digest: Some("sha256:abc".into()) };
        assert_eq!(r.full_reference(), "docker.io/library/nginx@sha256:abc");
    }

    // --- Enum serialization round-trips ---

    #[test]
    fn test_network_mode_all_variants_roundtrip() {
        for json in [r#""Host""#, r#""Bridge""#, r#""None""#] {
            let _: NetworkMode = serde_json::from_str(json).unwrap();
        }
        let modes: Vec<NetworkMode> = vec![NetworkMode::Host, NetworkMode::Bridge, NetworkMode::None];
        for m in modes {
            let s = serde_json::to_string(&m).unwrap();
            let _: NetworkMode = serde_json::from_str(&s).unwrap();
        }
    }

    #[test]
    fn test_network_mode_default_is_bridge() {
        assert!(matches!(NetworkMode::default(), NetworkMode::Bridge));
    }

    #[test]
    fn test_restart_policy_all_variants_roundtrip() {
        let policies: Vec<RestartPolicy> = vec![
            RestartPolicy::Never,
            RestartPolicy::Always,
            RestartPolicy::OnFailure { max_retries: 5 },
        ];
        for p in policies {
            let s = serde_json::to_string(&p).unwrap();
            let _: RestartPolicy = serde_json::from_str(&s).unwrap();
        }
    }

    #[test]
    fn test_restart_policy_default_is_never() {
        assert!(matches!(RestartPolicy::default(), RestartPolicy::Never));
    }

    #[test]
    fn test_mount_type_all_variants_roundtrip() {
        for mt in [MountType::Bind, MountType::Volume, MountType::Tmpfs] {
            let s = serde_json::to_string(&mt).unwrap();
            let _: MountType = serde_json::from_str(&s).unwrap();
        }
    }

    #[test]
    fn test_container_status_all_variants_roundtrip() {
        let variants = vec![
            ContainerStatus::Created,
            ContainerStatus::Running,
            ContainerStatus::Paused,
            ContainerStatus::Stopped,
            ContainerStatus::Failed("oom killed".into()),
        ];
        for s in variants {
            let json = serde_json::to_string(&s).unwrap();
            let back: ContainerStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_container_status_failed_preserves_message() {
        let s = ContainerStatus::Failed("exit code 137".into());
        let json = serde_json::to_string(&s).unwrap();
        let back: ContainerStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ContainerStatus::Failed("exit code 137".into()));
    }

    // --- ResourceLimits boundary values ---

    #[test]
    fn test_resource_limits_zero_values() {
        let r = ResourceLimits { cpu_shares: Some(0), cpu_quota: Some(0), memory_limit: Some(0), pids_limit: Some(0) };
        let json = serde_json::to_string(&r).unwrap();
        let back: ResourceLimits = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cpu_shares, Some(0));
        assert_eq!(back.memory_limit, Some(0));
        assert_eq!(back.pids_limit, Some(0));
    }

    #[test]
    fn test_resource_limits_max_values() {
        let r = ResourceLimits { cpu_shares: Some(u64::MAX), cpu_quota: Some(i64::MAX), memory_limit: Some(u64::MAX), pids_limit: Some(u64::MAX) };
        let json = serde_json::to_string(&r).unwrap();
        let back: ResourceLimits = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cpu_shares, Some(u64::MAX));
        assert_eq!(back.memory_limit, Some(u64::MAX));
    }

    // --- Mount struct ---

    #[test]
    fn test_mount_serialization() {
        let m = Mount {
            source: "/host/path".into(),
            destination: "/container/path".into(),
            read_only: true,
            mount_type: MountType::Bind,
            propagation: crate::models::MountPropagation::Private,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("host/path"));
        assert!(json.contains("Bind"));
    }
}

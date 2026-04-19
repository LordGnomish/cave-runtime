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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MountType {
    Bind,
    Volume,
    Tmpfs,
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
}

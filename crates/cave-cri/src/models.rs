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

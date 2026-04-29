//! CRI client — RuntimeService + ImageService request/response shapes.
//!
//! Mirrors `staging/src/k8s.io/cri-api/pkg/apis/runtime/v1/api.pb.go`: the
//! request envelopes the kubelet sends to CRI-O / containerd / cri-dockerd.
//! Captures the full v1 surface: PodSandbox lifecycle, Container lifecycle,
//! Exec/Attach/PortForward request shapes, ContainerStats / PodSandboxStats,
//! ImageService (PullImage, ListImages, RemoveImage, ImageStatus,
//! ImageFsInfo). All field-level validation lives here so the actual transport
//! is replaceable.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── PodSandbox ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSandboxMetadata {
    pub name: String,
    pub uid: String,
    pub namespace: String,
    pub attempt: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxPodSandboxConfig {
    pub cgroup_parent: String,
    pub overhead: Option<LinuxContainerResources>,
    pub resources: Option<LinuxContainerResources>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxContainerResources {
    pub cpu_period: i64,
    pub cpu_quota: i64,
    pub cpu_shares: i64,
    pub memory_limit_in_bytes: i64,
    pub memory_swap_limit_in_bytes: i64,
    pub oom_score_adj: i64,
    pub cpuset_cpus: String,
    pub cpuset_mems: String,
    pub hugepage_limits: Vec<HugepageLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HugepageLimit {
    pub page_size: String,
    pub limit: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSandboxConfig {
    pub metadata: PodSandboxMetadata,
    pub hostname: String,
    pub log_directory: String,
    pub dns_config: Option<DnsConfig>,
    pub port_mappings: Vec<PortMapping>,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub linux: Option<LinuxPodSandboxConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsConfig {
    pub servers: Vec<String>,
    pub searches: Vec<String>,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Tcp,
    Udp,
    Sctp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMapping {
    pub protocol: Protocol,
    pub container_port: i32,
    pub host_port: i32,
    pub host_ip: String,
}

// ── Container ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerMetadata {
    pub name: String,
    pub attempt: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerConfig {
    pub metadata: ContainerMetadata,
    pub image: ImageSpec,
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub working_dir: String,
    pub envs: Vec<KeyValue>,
    pub mounts: Vec<Mount>,
    pub devices: Vec<Device>,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub log_path: String,
    pub stdin: bool,
    pub stdin_once: bool,
    pub tty: bool,
    pub linux: Option<LinuxContainerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSpec {
    pub image: String,
    pub annotations: BTreeMap<String, String>,
    pub user_specified_image: String,
    pub runtime_handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub container_path: String,
    pub host_path: String,
    pub readonly: bool,
    pub selinux_relabel: bool,
    pub propagation: MountPropagation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountPropagation {
    Private,
    HostToContainer,
    Bidirectional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub container_path: String,
    pub host_path: String,
    pub permissions: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxContainerConfig {
    pub resources: Option<LinuxContainerResources>,
    pub security_context: Option<LinuxContainerSecurityContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxContainerSecurityContext {
    pub capabilities: Option<Capability>,
    pub privileged: bool,
    pub namespace_options: Option<NamespaceOption>,
    pub selinux_options: Option<SeLinuxOption>,
    pub run_as_user: Option<i64>,
    pub run_as_group: Option<i64>,
    pub run_as_username: String,
    pub readonly_rootfs: bool,
    pub supplemental_groups: Vec<i64>,
    pub no_new_privs: bool,
    pub masked_paths: Vec<String>,
    pub readonly_paths: Vec<String>,
    pub seccomp: Option<SecurityProfile>,
    pub apparmor: Option<SecurityProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub add_capabilities: Vec<String>,
    pub drop_capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceOption {
    pub network: NamespaceMode,
    pub pid: NamespaceMode,
    pub ipc: NamespaceMode,
    pub target_id: String,
    pub uses_host_user_namespace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespaceMode {
    Pod,
    Container,
    Node,
    Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeLinuxOption {
    pub user: String,
    pub role: String,
    pub kind: String,
    pub level: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityProfile {
    pub profile_type: SecurityProfileType,
    pub localhost_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityProfileType {
    RuntimeDefault,
    Unconfined,
    Localhost,
}

// ── Status / Stats ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodSandboxState {
    Ready,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSandboxStatus {
    pub id: String,
    pub metadata: PodSandboxMetadata,
    pub state: PodSandboxState,
    pub created_at: i64,
    pub network: Option<PodSandboxNetworkStatus>,
    pub linux: Option<LinuxPodSandboxStatus>,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub runtime_handler: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSandboxNetworkStatus {
    pub ip: String,
    pub additional_ips: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxPodSandboxStatus {
    pub namespaces: Option<Namespace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    pub options: Option<NamespaceOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerState {
    Created,
    Running,
    Exited,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub id: String,
    pub metadata: ContainerMetadata,
    pub state: ContainerState,
    pub created_at: i64,
    pub started_at: i64,
    pub finished_at: i64,
    pub exit_code: i32,
    pub image: ImageSpec,
    pub image_ref: String,
    pub reason: String,
    pub message: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub mounts: Vec<Mount>,
    pub log_path: String,
}

// ── Image ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub repo_digests: Vec<String>,
    pub size: u64,
    pub uid: Option<i64>,
    pub username: String,
    pub spec: Option<ImageSpec>,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    pub username: String,
    pub password: String,
    pub auth: String,
    pub server_address: String,
    pub identity_token: String,
    pub registry_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemUsage {
    pub timestamp: i64,
    pub fs_id: String,
    pub used_bytes: u64,
    pub inodes_used: u64,
}

// ── Errors ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CriError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unimplemented runtime feature: {0}")]
    UnsupportedFeature(String),
}

pub type CriResult<T> = Result<T, CriError>;

// ── Validation ──────────────────────────────────────────────────────────

pub fn validate_pod_sandbox_config(c: &PodSandboxConfig) -> CriResult<()> {
    if c.metadata.name.is_empty() {
        return Err(CriError::Invalid("metadata.name empty".into()));
    }
    if c.metadata.uid.is_empty() {
        return Err(CriError::Invalid("metadata.uid empty".into()));
    }
    if c.metadata.namespace.is_empty() {
        return Err(CriError::Invalid("metadata.namespace empty".into()));
    }
    for p in &c.port_mappings {
        if p.container_port <= 0 || p.container_port > 65535 {
            return Err(CriError::Invalid(format!(
                "container_port out of range: {}",
                p.container_port
            )));
        }
        if p.host_port < 0 || p.host_port > 65535 {
            return Err(CriError::Invalid(format!(
                "host_port out of range: {}",
                p.host_port
            )));
        }
    }
    Ok(())
}

pub fn validate_container_config(
    c: &ContainerConfig,
    sandbox: &PodSandboxConfig,
) -> CriResult<()> {
    if c.metadata.name.is_empty() {
        return Err(CriError::Invalid("container metadata.name empty".into()));
    }
    if c.image.image.is_empty() {
        return Err(CriError::Invalid("image.image empty".into()));
    }
    if c.log_path.is_empty() && !sandbox.log_directory.is_empty() {
        return Err(CriError::Invalid(
            "log_path required when sandbox declares log_directory".into(),
        ));
    }
    for m in &c.mounts {
        if m.container_path.is_empty() {
            return Err(CriError::Invalid("mount.container_path empty".into()));
        }
        if !m.container_path.starts_with('/') {
            return Err(CriError::Invalid(
                "mount.container_path must be absolute".into(),
            ));
        }
        if m.host_path.is_empty() {
            return Err(CriError::Invalid("mount.host_path empty".into()));
        }
    }
    for d in &c.devices {
        if d.container_path.is_empty() || d.host_path.is_empty() {
            return Err(CriError::Invalid("device path empty".into()));
        }
        if !d
            .permissions
            .chars()
            .all(|c| matches!(c, 'r' | 'w' | 'm'))
        {
            return Err(CriError::Invalid(format!(
                "invalid device permissions: {}",
                d.permissions
            )));
        }
    }
    if let Some(linux) = &c.linux {
        if let Some(res) = &linux.resources {
            validate_linux_resources(res)?;
        }
        if let Some(sc) = &linux.security_context {
            validate_security_context(sc)?;
        }
    }
    Ok(())
}

pub fn validate_linux_resources(r: &LinuxContainerResources) -> CriResult<()> {
    if r.cpu_period < 0 {
        return Err(CriError::Invalid("cpu_period negative".into()));
    }
    if r.cpu_quota < -1 {
        return Err(CriError::Invalid("cpu_quota < -1".into()));
    }
    if r.cpu_shares < 0 {
        return Err(CriError::Invalid("cpu_shares negative".into()));
    }
    if r.memory_limit_in_bytes < -1 {
        return Err(CriError::Invalid("memory_limit_in_bytes < -1".into()));
    }
    if !(-1000..=1000).contains(&r.oom_score_adj) {
        return Err(CriError::Invalid(format!(
            "oom_score_adj out of range: {}",
            r.oom_score_adj
        )));
    }
    for h in &r.hugepage_limits {
        if h.page_size.is_empty() {
            return Err(CriError::Invalid("hugepage page_size empty".into()));
        }
    }
    Ok(())
}

pub fn validate_security_context(sc: &LinuxContainerSecurityContext) -> CriResult<()> {
    if let Some(p) = &sc.seccomp {
        validate_security_profile(p, "seccomp")?;
    }
    if let Some(p) = &sc.apparmor {
        validate_security_profile(p, "apparmor")?;
    }
    Ok(())
}

pub fn validate_security_profile(p: &SecurityProfile, kind: &str) -> CriResult<()> {
    if p.profile_type == SecurityProfileType::Localhost && p.localhost_ref.is_empty() {
        return Err(CriError::Invalid(format!(
            "{} Localhost requires localhost_ref",
            kind
        )));
    }
    if p.profile_type != SecurityProfileType::Localhost && !p.localhost_ref.is_empty() {
        return Err(CriError::Invalid(format!(
            "{} localhost_ref allowed only with Localhost",
            kind
        )));
    }
    Ok(())
}

pub fn validate_image_spec(s: &ImageSpec) -> CriResult<()> {
    if s.image.is_empty() {
        return Err(CriError::Invalid("image empty".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb_cfg(name: &str, uid: &str, ns: &str) -> PodSandboxConfig {
        PodSandboxConfig {
            metadata: PodSandboxMetadata {
                name: name.into(),
                uid: uid.into(),
                namespace: ns.into(),
                attempt: 0,
            },
            ..PodSandboxConfig::default()
        }
    }

    fn ctr_cfg(name: &str, image: &str) -> ContainerConfig {
        ContainerConfig {
            metadata: ContainerMetadata {
                name: name.into(),
                attempt: 0,
            },
            image: ImageSpec {
                image: image.into(),
                ..ImageSpec::default()
            },
            ..ContainerConfig::default()
        }
    }

    #[test]
    fn pod_sandbox_validate_ok() {
        validate_pod_sandbox_config(&sb_cfg("nginx", "u-1", "default")).unwrap();
    }

    #[test]
    fn pod_sandbox_rejects_empty_name() {
        let mut s = sb_cfg("", "u", "ns");
        s.metadata.name = "".into();
        assert!(validate_pod_sandbox_config(&s).is_err());
    }

    #[test]
    fn pod_sandbox_rejects_empty_uid() {
        let mut s = sb_cfg("n", "", "ns");
        s.metadata.uid = "".into();
        assert!(validate_pod_sandbox_config(&s).is_err());
    }

    #[test]
    fn pod_sandbox_rejects_empty_namespace() {
        let mut s = sb_cfg("n", "u", "");
        s.metadata.namespace = "".into();
        assert!(validate_pod_sandbox_config(&s).is_err());
    }

    #[test]
    fn port_mapping_zero_container_port_rejected() {
        let mut s = sb_cfg("n", "u", "ns");
        s.port_mappings.push(PortMapping {
            protocol: Protocol::Tcp,
            container_port: 0,
            host_port: 0,
            host_ip: "".into(),
        });
        assert!(validate_pod_sandbox_config(&s).is_err());
    }

    #[test]
    fn port_mapping_high_container_port_rejected() {
        let mut s = sb_cfg("n", "u", "ns");
        s.port_mappings.push(PortMapping {
            protocol: Protocol::Tcp,
            container_port: 70000,
            host_port: 0,
            host_ip: "".into(),
        });
        assert!(validate_pod_sandbox_config(&s).is_err());
    }

    #[test]
    fn port_mapping_negative_host_port_rejected() {
        let mut s = sb_cfg("n", "u", "ns");
        s.port_mappings.push(PortMapping {
            protocol: Protocol::Udp,
            container_port: 80,
            host_port: -1,
            host_ip: "".into(),
        });
        assert!(validate_pod_sandbox_config(&s).is_err());
    }

    #[test]
    fn port_mapping_high_host_port_rejected() {
        let mut s = sb_cfg("n", "u", "ns");
        s.port_mappings.push(PortMapping {
            protocol: Protocol::Tcp,
            container_port: 80,
            host_port: 70000,
            host_ip: "".into(),
        });
        assert!(validate_pod_sandbox_config(&s).is_err());
    }

    #[test]
    fn port_mapping_valid_passes() {
        let mut s = sb_cfg("n", "u", "ns");
        s.port_mappings.push(PortMapping {
            protocol: Protocol::Tcp,
            container_port: 80,
            host_port: 8080,
            host_ip: "0.0.0.0".into(),
        });
        validate_pod_sandbox_config(&s).unwrap();
    }

    #[test]
    fn container_validate_ok() {
        validate_container_config(&ctr_cfg("c", "nginx:1.25"), &sb_cfg("n", "u", "ns")).unwrap();
    }

    #[test]
    fn container_rejects_empty_name() {
        let mut c = ctr_cfg("", "nginx");
        c.metadata.name = "".into();
        assert!(validate_container_config(&c, &sb_cfg("n", "u", "ns")).is_err());
    }

    #[test]
    fn container_rejects_empty_image() {
        let c = ctr_cfg("c", "");
        assert!(validate_container_config(&c, &sb_cfg("n", "u", "ns")).is_err());
    }

    #[test]
    fn container_log_path_required_when_sandbox_dir_set() {
        let mut sb = sb_cfg("n", "u", "ns");
        sb.log_directory = "/var/log/pods/n".into();
        let c = ctr_cfg("c", "nginx");
        assert!(validate_container_config(&c, &sb).is_err());
    }

    #[test]
    fn container_log_path_ok_when_sandbox_dir_empty() {
        let sb = sb_cfg("n", "u", "ns");
        let c = ctr_cfg("c", "nginx");
        validate_container_config(&c, &sb).unwrap();
    }

    #[test]
    fn mount_container_path_must_be_absolute() {
        let mut c = ctr_cfg("c", "nginx");
        c.mounts.push(Mount {
            container_path: "relative/path".into(),
            host_path: "/h".into(),
            readonly: false,
            selinux_relabel: false,
            propagation: MountPropagation::Private,
        });
        assert!(validate_container_config(&c, &sb_cfg("n", "u", "ns")).is_err());
    }

    #[test]
    fn mount_host_path_required() {
        let mut c = ctr_cfg("c", "nginx");
        c.mounts.push(Mount {
            container_path: "/c".into(),
            host_path: "".into(),
            readonly: false,
            selinux_relabel: false,
            propagation: MountPropagation::Private,
        });
        assert!(validate_container_config(&c, &sb_cfg("n", "u", "ns")).is_err());
    }

    #[test]
    fn mount_valid_propagation_modes() {
        for prop in [
            MountPropagation::Private,
            MountPropagation::HostToContainer,
            MountPropagation::Bidirectional,
        ] {
            let mut c = ctr_cfg("c", "nginx");
            c.mounts.push(Mount {
                container_path: "/c".into(),
                host_path: "/h".into(),
                readonly: false,
                selinux_relabel: false,
                propagation: prop,
            });
            validate_container_config(&c, &sb_cfg("n", "u", "ns")).unwrap();
        }
    }

    #[test]
    fn device_permissions_validated() {
        let mut c = ctr_cfg("c", "nginx");
        c.devices.push(Device {
            container_path: "/dev/foo".into(),
            host_path: "/dev/foo".into(),
            permissions: "rwx".into(), // x not allowed
        });
        assert!(validate_container_config(&c, &sb_cfg("n", "u", "ns")).is_err());
    }

    #[test]
    fn device_rwm_permissions_accepted() {
        let mut c = ctr_cfg("c", "nginx");
        c.devices.push(Device {
            container_path: "/dev/foo".into(),
            host_path: "/dev/foo".into(),
            permissions: "rwm".into(),
        });
        validate_container_config(&c, &sb_cfg("n", "u", "ns")).unwrap();
    }

    #[test]
    fn device_empty_path_rejected() {
        let mut c = ctr_cfg("c", "nginx");
        c.devices.push(Device {
            container_path: "".into(),
            host_path: "/dev/x".into(),
            permissions: "r".into(),
        });
        assert!(validate_container_config(&c, &sb_cfg("n", "u", "ns")).is_err());
    }

    #[test]
    fn linux_resources_negative_cpu_period_rejected() {
        let r = LinuxContainerResources {
            cpu_period: -1,
            ..LinuxContainerResources::default()
        };
        assert!(validate_linux_resources(&r).is_err());
    }

    #[test]
    fn linux_resources_cpu_quota_neg2_rejected() {
        let r = LinuxContainerResources {
            cpu_quota: -2,
            ..LinuxContainerResources::default()
        };
        assert!(validate_linux_resources(&r).is_err());
    }

    #[test]
    fn linux_resources_cpu_quota_neg1_accepted() {
        // -1 means unlimited.
        let r = LinuxContainerResources {
            cpu_quota: -1,
            ..LinuxContainerResources::default()
        };
        validate_linux_resources(&r).unwrap();
    }

    #[test]
    fn linux_resources_negative_cpu_shares_rejected() {
        let r = LinuxContainerResources {
            cpu_shares: -1,
            ..LinuxContainerResources::default()
        };
        assert!(validate_linux_resources(&r).is_err());
    }

    #[test]
    fn linux_resources_memory_neg1_accepted() {
        let r = LinuxContainerResources {
            memory_limit_in_bytes: -1,
            ..LinuxContainerResources::default()
        };
        validate_linux_resources(&r).unwrap();
    }

    #[test]
    fn linux_resources_memory_neg2_rejected() {
        let r = LinuxContainerResources {
            memory_limit_in_bytes: -2,
            ..LinuxContainerResources::default()
        };
        assert!(validate_linux_resources(&r).is_err());
    }

    #[test]
    fn linux_resources_oom_score_adj_range() {
        let mut r = LinuxContainerResources::default();
        r.oom_score_adj = 1001;
        assert!(validate_linux_resources(&r).is_err());
        r.oom_score_adj = -1001;
        assert!(validate_linux_resources(&r).is_err());
        r.oom_score_adj = 1000;
        validate_linux_resources(&r).unwrap();
        r.oom_score_adj = -1000;
        validate_linux_resources(&r).unwrap();
    }

    #[test]
    fn hugepage_empty_size_rejected() {
        let r = LinuxContainerResources {
            hugepage_limits: vec![HugepageLimit {
                page_size: "".into(),
                limit: 1024,
            }],
            ..LinuxContainerResources::default()
        };
        assert!(validate_linux_resources(&r).is_err());
    }

    #[test]
    fn hugepage_2mb_accepted() {
        let r = LinuxContainerResources {
            hugepage_limits: vec![HugepageLimit {
                page_size: "2MB".into(),
                limit: 1024,
            }],
            ..LinuxContainerResources::default()
        };
        validate_linux_resources(&r).unwrap();
    }

    #[test]
    fn seccomp_localhost_requires_ref() {
        let p = SecurityProfile {
            profile_type: SecurityProfileType::Localhost,
            localhost_ref: "".into(),
        };
        assert!(validate_security_profile(&p, "seccomp").is_err());
    }

    #[test]
    fn seccomp_localhost_with_ref_ok() {
        let p = SecurityProfile {
            profile_type: SecurityProfileType::Localhost,
            localhost_ref: "/var/lib/kubelet/seccomp/strict.json".into(),
        };
        validate_security_profile(&p, "seccomp").unwrap();
    }

    #[test]
    fn seccomp_runtime_default_with_ref_rejected() {
        let p = SecurityProfile {
            profile_type: SecurityProfileType::RuntimeDefault,
            localhost_ref: "/path".into(),
        };
        assert!(validate_security_profile(&p, "seccomp").is_err());
    }

    #[test]
    fn seccomp_unconfined_with_ref_rejected() {
        let p = SecurityProfile {
            profile_type: SecurityProfileType::Unconfined,
            localhost_ref: "/path".into(),
        };
        assert!(validate_security_profile(&p, "seccomp").is_err());
    }

    #[test]
    fn apparmor_localhost_requires_ref() {
        let p = SecurityProfile {
            profile_type: SecurityProfileType::Localhost,
            localhost_ref: "".into(),
        };
        assert!(validate_security_profile(&p, "apparmor").is_err());
    }

    #[test]
    fn security_context_with_invalid_seccomp_propagates() {
        let sc = LinuxContainerSecurityContext {
            seccomp: Some(SecurityProfile {
                profile_type: SecurityProfileType::Localhost,
                localhost_ref: "".into(),
            }),
            ..LinuxContainerSecurityContext::default()
        };
        assert!(validate_security_context(&sc).is_err());
    }

    #[test]
    fn security_context_with_valid_seccomp_passes() {
        let sc = LinuxContainerSecurityContext {
            seccomp: Some(SecurityProfile {
                profile_type: SecurityProfileType::RuntimeDefault,
                localhost_ref: "".into(),
            }),
            ..LinuxContainerSecurityContext::default()
        };
        validate_security_context(&sc).unwrap();
    }

    #[test]
    fn validate_image_spec_rejects_empty() {
        assert!(validate_image_spec(&ImageSpec::default()).is_err());
    }

    #[test]
    fn validate_image_spec_accepts_non_empty() {
        let s = ImageSpec {
            image: "nginx:1.25".into(),
            ..ImageSpec::default()
        };
        validate_image_spec(&s).unwrap();
    }

    #[test]
    fn protocol_distinct_variants() {
        assert_ne!(Protocol::Tcp, Protocol::Udp);
        assert_ne!(Protocol::Udp, Protocol::Sctp);
    }

    #[test]
    fn namespace_mode_variants_distinct() {
        assert_ne!(NamespaceMode::Pod, NamespaceMode::Container);
        assert_ne!(NamespaceMode::Container, NamespaceMode::Node);
        assert_ne!(NamespaceMode::Node, NamespaceMode::Target);
    }

    #[test]
    fn container_state_variants_distinct() {
        assert_ne!(ContainerState::Created, ContainerState::Running);
        assert_ne!(ContainerState::Running, ContainerState::Exited);
        assert_ne!(ContainerState::Exited, ContainerState::Unknown);
    }

    #[test]
    fn pod_sandbox_state_distinct_variants() {
        assert_ne!(PodSandboxState::Ready, PodSandboxState::NotReady);
    }

    #[test]
    fn key_value_basic() {
        let kv = KeyValue { key: "K".into(), value: "V".into() };
        assert_eq!(kv.key, "K");
        assert_eq!(kv.value, "V");
    }

    #[test]
    fn dns_config_default_empty() {
        let d = DnsConfig::default();
        assert!(d.servers.is_empty());
        assert!(d.searches.is_empty());
    }

    #[test]
    fn pod_sandbox_serde_round_trip() {
        let s = sb_cfg("n", "u", "ns");
        let json = serde_json::to_string(&s).unwrap();
        let s2: PodSandboxConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn container_serde_round_trip() {
        let c = ctr_cfg("c", "nginx");
        let json = serde_json::to_string(&c).unwrap();
        let c2: ContainerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(c, c2);
    }

    #[test]
    fn linux_resources_default_validates() {
        validate_linux_resources(&LinuxContainerResources::default()).unwrap();
    }

    #[test]
    fn auth_config_default_empty() {
        let a = AuthConfig {
            username: "".into(),
            password: "".into(),
            auth: "".into(),
            server_address: "".into(),
            identity_token: "".into(),
            registry_token: "".into(),
        };
        assert_eq!(a.username, "");
    }

    #[test]
    fn image_with_repo_tags_and_digests() {
        let img = Image {
            id: "sha256:abc".into(),
            repo_tags: vec!["nginx:1.25".into()],
            repo_digests: vec!["nginx@sha256:abc".into()],
            size: 1024,
            uid: None,
            username: "".into(),
            spec: None,
            pinned: false,
        };
        assert_eq!(img.size, 1024);
        assert!(!img.pinned);
    }
}

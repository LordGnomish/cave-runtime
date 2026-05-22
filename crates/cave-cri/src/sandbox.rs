// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pod sandbox lifecycle — RunPodSandbox / StopPodSandbox.
//!
//! Implements the containerd `pkg/cri/server/sandbox_run.go` and
//! `sandbox_stop.go` flow:
//!
//! 1. Allocate sandbox namespaces (net, ipc, uts, mount) under
//!    `/var/run/cave/sandboxes/<id>/ns/<kind>`.
//! 2. Materialise a `pause` container — the long-lived idle process that
//!    keeps the sandbox namespaces alive while child containers come and
//!    go.
//! 3. Generate the iptables-style portmap entries for any
//!    `port_mappings` declared on the sandbox spec.
//! 4. Tear all of the above down on stop.

use crate::error::{CriError, CriResult};
use crate::models::{PortMapping, Sandbox, SandboxSpec, SandboxState, UserNamespaceMode};
use crate::paths;
use crate::userns::{UserNamespace, UserNsAllocator};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Default image used as the sandbox's pause container. Mirrors
/// `kubernetes/build/pause/Dockerfile` — a minimal binary that sleeps
/// forever holding the sandbox namespaces open.
pub const DEFAULT_PAUSE_IMAGE: &str = "registry.k8s.io/pause:3.9";

/// Long-lived idle process keeping the sandbox namespaces alive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseContainer {
    pub sandbox_id: Uuid,
    pub image: String,
    pub pid: Option<u32>,
}

impl PauseContainer {
    pub fn new(sandbox_id: Uuid) -> Self {
        Self {
            sandbox_id,
            image: DEFAULT_PAUSE_IMAGE.into(),
            pid: None,
        }
    }
}

/// Per-sandbox namespace paths (bind-mounted from `/proc/self/ns/<kind>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxNamespaces {
    pub network: PathBuf,
    pub ipc: PathBuf,
    pub uts: PathBuf,
    pub mount: PathBuf,
}

impl SandboxNamespaces {
    /// Compute the namespace paths for a sandbox under
    /// `<root>/sandboxes/<id>/ns/`.
    pub fn for_sandbox(sandbox_id: Uuid) -> Self {
        let root = paths::root()
            .join("sandboxes")
            .join(sandbox_id.to_string())
            .join("ns");
        Self {
            network: root.join("net"),
            ipc: root.join("ipc"),
            uts: root.join("uts"),
            mount: root.join("mnt"),
        }
    }

    /// Create the namespace bind-mount targets (empty files on disk).
    pub fn create(&self) -> CriResult<()> {
        for p in [&self.network, &self.ipc, &self.uts, &self.mount] {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(CriError::Io)?;
            }
            // Touch the file (the real bind-mount happens at runtime).
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(p)
                .map_err(CriError::Io)?;
        }
        Ok(())
    }

    /// Remove the namespace targets.
    pub fn teardown(&self) -> CriResult<()> {
        for p in [&self.network, &self.ipc, &self.uts, &self.mount] {
            if p.exists() {
                std::fs::remove_file(p).ok();
            }
        }
        // Best-effort: drop the per-sandbox dir if empty.
        if let Some(parent) = self.network.parent() {
            std::fs::remove_dir(parent).ok();
            if let Some(grand) = parent.parent() {
                std::fs::remove_dir(grand).ok();
            }
        }
        Ok(())
    }
}

/// Outcome of running a sandbox: the sandbox object plus the per-sandbox
/// resources (pause container, namespaces, validated port mappings,
/// optional KEP-127 user namespace).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSandboxResult {
    pub sandbox: Sandbox,
    pub pause: PauseContainer,
    pub namespaces: SandboxNamespaces,
    pub port_mappings: Vec<PortMapping>,
    pub user_namespace: UserNamespace,
}

/// Allocate a sandbox: create namespaces, the pause container, apply
/// validated port mappings, and (optionally) reserve a KEP-127 user
/// namespace from `userns_allocator`.
pub fn run_pod_sandbox(
    spec: SandboxSpec,
    userns_allocator: Option<&UserNsAllocator>,
) -> CriResult<RunSandboxResult> {
    let sandbox_id = Uuid::new_v4();

    // Validate port mappings before doing any side effects.
    for pm in &spec.port_mappings {
        validate_port_mapping(pm)?;
    }

    let namespaces = SandboxNamespaces::for_sandbox(sandbox_id);
    namespaces.create()?;

    let user_namespace = match (spec.user_namespace_mode.clone(), userns_allocator) {
        (UserNamespaceMode::Pod, Some(alloc)) => {
            alloc.allocate_namespace().map_err(CriError::Sandbox)?
        }
        (UserNamespaceMode::Pod, None) => {
            return Err(CriError::Sandbox(
                "user_namespace_mode=Pod but no UserNsAllocator configured".into(),
            ));
        }
        (UserNamespaceMode::Host, _) => UserNamespace::host_passthrough(),
    };

    let pause = PauseContainer::new(sandbox_id);
    let sandbox = Sandbox {
        id: sandbox_id,
        spec: spec.clone(),
        state: SandboxState::Ready,
        created_at: Utc::now(),
        // Kubelet's IPAM gives each pod a /32 from the pod CIDR; we pick a
        // deterministic placeholder so tests can assert.
        network_ip: Some(allocate_pod_ip(sandbox_id)),
    };

    Ok(RunSandboxResult {
        sandbox,
        pause,
        namespaces,
        port_mappings: spec.port_mappings,
        user_namespace,
    })
}

/// Stop a sandbox: tear down namespaces; caller is responsible for
/// stopping member containers and the pause process.
pub fn stop_pod_sandbox(sandbox_id: Uuid) -> CriResult<()> {
    let ns = SandboxNamespaces::for_sandbox(sandbox_id);
    ns.teardown()?;
    Ok(())
}

/// Validate a single port mapping. Mirrors the checks containerd does in
/// `pkg/cri/server/sandbox_portforward.go`.
pub fn validate_port_mapping(pm: &PortMapping) -> CriResult<()> {
    if pm.container_port == 0 {
        return Err(CriError::Sandbox("container_port must be non-zero".into()));
    }
    if pm.host_port == 0 {
        return Err(CriError::Sandbox("host_port must be non-zero".into()));
    }
    if !matches!(pm.protocol.to_uppercase().as_str(), "TCP" | "UDP" | "SCTP") {
        return Err(CriError::Sandbox(format!(
            "unsupported port protocol: {}",
            pm.protocol
        )));
    }
    if let Some(ip) = &pm.host_ip {
        if !ip.is_empty() && ip.parse::<std::net::IpAddr>().is_err() {
            return Err(CriError::Sandbox(format!("invalid host_ip: {}", ip)));
        }
    }
    Ok(())
}

/// Render an iptables `-t nat` PREROUTING rule for one port mapping.
/// Used by the network-plugin shim and exposed for testing.
pub fn render_iptables_rule(sandbox_ip: &str, pm: &PortMapping) -> String {
    let host_ip = pm.host_ip.as_deref().unwrap_or("0.0.0.0");
    format!(
        "-A PREROUTING -p {proto} -d {host_ip} --dport {host_port} -j DNAT --to-destination {pod}:{container_port}",
        proto = pm.protocol.to_lowercase(),
        host_ip = host_ip,
        host_port = pm.host_port,
        pod = sandbox_ip,
        container_port = pm.container_port,
    )
}

/// Deterministic placeholder IP allocator — picks `10.244.<lo>.<hi>` from
/// the sandbox UUID so tests can assert on the value without relying on
/// state.
fn allocate_pod_ip(sandbox_id: Uuid) -> String {
    let bytes = sandbox_id.as_bytes();
    let lo = bytes[0];
    let hi = bytes[1].max(2); // avoid the .0 / .1 (gateway) addresses
    format!("10.244.{}.{}", lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Once;

    static INIT_ROOT: Once = Once::new();
    fn ensure_test_root() {
        INIT_ROOT.call_once(|| {
            let dir = std::env::temp_dir().join(format!("cave-cri-sb-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("CAVE_ROOT_DIR", &dir);
        });
    }

    fn spec(name: &str, ports: Vec<PortMapping>) -> SandboxSpec {
        SandboxSpec {
            name: name.into(),
            namespace: "default".into(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            hostname: Some(name.into()),
            dns_config: None,
            port_mappings: ports,
            log_directory: None,
            cgroup_parent: None,
            runtime_handler: None,
            user_namespace_mode: crate::models::UserNamespaceMode::Host,
        }
    }

    fn pm(host: u16, container: u16, proto: &str) -> PortMapping {
        PortMapping {
            protocol: proto.into(),
            container_port: container,
            host_port: host,
            host_ip: None,
        }
    }

    // ── PauseContainer ───────────────────────────────────────────────────────

    #[test]
    fn pause_container_uses_default_image() {
        let p = PauseContainer::new(Uuid::new_v4());
        assert_eq!(p.image, DEFAULT_PAUSE_IMAGE);
        assert!(p.pid.is_none());
    }

    #[test]
    fn pause_container_remembers_sandbox_id() {
        let id = Uuid::new_v4();
        let p = PauseContainer::new(id);
        assert_eq!(p.sandbox_id, id);
    }

    // ── SandboxNamespaces ────────────────────────────────────────────────────

    #[test]
    fn namespace_paths_have_expected_layout() {
        ensure_test_root();
        let id = Uuid::new_v4();
        let ns = SandboxNamespaces::for_sandbox(id);
        assert!(ns.network.to_string_lossy().ends_with("net"));
        assert!(ns.ipc.to_string_lossy().ends_with("ipc"));
        assert!(ns.uts.to_string_lossy().ends_with("uts"));
        assert!(ns.mount.to_string_lossy().ends_with("mnt"));
        assert!(ns.network.to_string_lossy().contains(&id.to_string()));
    }

    #[test]
    fn namespace_create_then_teardown_idempotent() {
        ensure_test_root();
        let ns = SandboxNamespaces::for_sandbox(Uuid::new_v4());
        ns.create().unwrap();
        assert!(ns.network.exists());
        ns.teardown().unwrap();
        assert!(!ns.network.exists());
        // Second teardown is a no-op.
        ns.teardown().unwrap();
    }

    // ── run_pod_sandbox ─────────────────────────────────────────────────────

    #[test]
    fn run_pod_sandbox_marks_ready_and_allocates_ip() {
        ensure_test_root();
        let r = run_pod_sandbox(spec("p1", vec![]), None).unwrap();
        assert_eq!(r.sandbox.state, SandboxState::Ready);
        assert!(r
            .sandbox
            .network_ip
            .as_deref()
            .unwrap()
            .starts_with("10.244."));
    }

    #[test]
    fn run_pod_sandbox_creates_namespace_files() {
        ensure_test_root();
        let r = run_pod_sandbox(spec("p2", vec![]), None).unwrap();
        assert!(r.namespaces.network.exists());
        assert!(r.namespaces.ipc.exists());
    }

    #[test]
    fn run_pod_sandbox_assigns_pause_container() {
        ensure_test_root();
        let r = run_pod_sandbox(spec("p3", vec![]), None).unwrap();
        assert_eq!(r.pause.sandbox_id, r.sandbox.id);
        assert_eq!(r.pause.image, DEFAULT_PAUSE_IMAGE);
    }

    #[test]
    fn run_pod_sandbox_keeps_port_mappings() {
        ensure_test_root();
        let r = run_pod_sandbox(spec("p4", vec![pm(8080, 80, "TCP")]), None).unwrap();
        assert_eq!(r.port_mappings.len(), 1);
        assert_eq!(r.port_mappings[0].host_port, 8080);
    }

    #[test]
    fn run_pod_sandbox_rejects_invalid_port() {
        let r = run_pod_sandbox(spec("p5", vec![pm(8080, 0, "TCP")]), None);
        assert!(r.is_err());
    }

    #[test]
    fn run_pod_sandbox_rejects_unsupported_proto() {
        let r = run_pod_sandbox(spec("p6", vec![pm(8080, 80, "ICMP")]), None);
        assert!(r.is_err());
    }

    #[test]
    fn run_pod_sandbox_accepts_udp() {
        ensure_test_root();
        let r = run_pod_sandbox(spec("p7", vec![pm(53, 53, "UDP")]), None).unwrap();
        assert_eq!(r.port_mappings[0].protocol, "UDP");
    }

    #[test]
    fn run_pod_sandbox_accepts_sctp() {
        ensure_test_root();
        let r = run_pod_sandbox(spec("p8", vec![pm(36412, 36412, "SCTP")]), None).unwrap();
        assert_eq!(r.port_mappings[0].protocol, "SCTP");
    }

    // ── stop_pod_sandbox ────────────────────────────────────────────────────

    #[test]
    fn stop_pod_sandbox_removes_namespace_files() {
        ensure_test_root();
        let r = run_pod_sandbox(spec("stop", vec![]), None).unwrap();
        let ns_path = r.namespaces.network.clone();
        assert!(ns_path.exists());
        stop_pod_sandbox(r.sandbox.id).unwrap();
        assert!(!ns_path.exists());
    }

    #[test]
    fn stop_pod_sandbox_unknown_id_is_idempotent() {
        ensure_test_root();
        // Should not error even if the sandbox was never created.
        stop_pod_sandbox(Uuid::new_v4()).unwrap();
    }

    // ── validate_port_mapping ────────────────────────────────────────────────

    #[test]
    fn validate_port_mapping_zero_container_port_errors() {
        assert!(validate_port_mapping(&pm(80, 0, "TCP")).is_err());
    }

    #[test]
    fn validate_port_mapping_zero_host_port_errors() {
        assert!(validate_port_mapping(&pm(0, 80, "TCP")).is_err());
    }

    #[test]
    fn validate_port_mapping_invalid_proto_errors() {
        let err = validate_port_mapping(&pm(80, 80, "QUIC")).unwrap_err();
        assert!(err.to_string().contains("QUIC"));
    }

    #[test]
    fn validate_port_mapping_valid_host_ip_ok() {
        let mut p = pm(80, 80, "TCP");
        p.host_ip = Some("192.168.1.1".into());
        assert!(validate_port_mapping(&p).is_ok());
    }

    #[test]
    fn validate_port_mapping_invalid_host_ip_errors() {
        let mut p = pm(80, 80, "TCP");
        p.host_ip = Some("not.an.ip".into());
        assert!(validate_port_mapping(&p).is_err());
    }

    #[test]
    fn validate_port_mapping_empty_host_ip_ok() {
        let mut p = pm(80, 80, "TCP");
        p.host_ip = Some(String::new());
        assert!(validate_port_mapping(&p).is_ok());
    }

    #[test]
    fn validate_port_mapping_proto_case_insensitive() {
        assert!(validate_port_mapping(&pm(80, 80, "tcp")).is_ok());
        assert!(validate_port_mapping(&pm(80, 80, "Udp")).is_ok());
    }

    // ── render_iptables_rule ────────────────────────────────────────────────

    #[test]
    fn iptables_rule_includes_dnat_to_pod() {
        let rule = render_iptables_rule("10.244.0.5", &pm(8080, 80, "TCP"));
        assert!(rule.contains("-p tcp"));
        assert!(rule.contains("--dport 8080"));
        assert!(rule.contains("--to-destination 10.244.0.5:80"));
        assert!(rule.contains("-d 0.0.0.0"));
    }

    #[test]
    fn iptables_rule_uses_specified_host_ip() {
        let mut p = pm(443, 443, "TCP");
        p.host_ip = Some("192.168.1.10".into());
        let rule = render_iptables_rule("10.244.0.7", &p);
        assert!(rule.contains("-d 192.168.1.10"));
    }

    #[test]
    fn iptables_rule_for_udp_uses_lowercase_proto() {
        let rule = render_iptables_rule("10.244.0.8", &pm(53, 53, "UDP"));
        assert!(rule.contains("-p udp"));
    }
}

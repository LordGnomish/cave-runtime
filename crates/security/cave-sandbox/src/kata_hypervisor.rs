// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: hypervisor abstraction adapted from kata-containers
// src/runtime/virtcontainers/hypervisor.go (Apache-2.0).
//! kata-runtime hypervisor abstraction.
//!
//! Three concrete backends: QEMU, cloud-hypervisor, Firecracker. We model the
//! configuration shape — actual VM spawn / KVM is OUT OF SCOPE.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Backend selector — `virtcontainers/hypervisor.go::HypervisorType`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HypervisorType {
    Qemu,
    CloudHypervisor,
    Firecracker,
    /// Mock — for unit tests; no kernel interaction.
    Mock,
}

impl HypervisorType {
    pub fn as_str(self) -> &'static str {
        match self {
            HypervisorType::Qemu => "qemu",
            HypervisorType::CloudHypervisor => "cloud-hypervisor",
            HypervisorType::Firecracker => "firecracker",
            HypervisorType::Mock => "mock",
        }
    }
}

/// Common config — `virtcontainers/hypervisor.go::HypervisorConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HypervisorConfig {
    pub kernel_path: String,
    pub initrd_path: Option<String>,
    pub image_path: Option<String>,
    pub firmware_path: Option<String>,
    pub kernel_params: String,
    pub default_vcpus: u32,
    pub default_max_vcpus: u32,
    pub default_mem_mib: u32,
    pub default_max_mem_mib: u32,
    pub block_device_driver: String,
    pub disable_nesting_checks: bool,
    pub enable_iommu: bool,
}

impl Default for HypervisorConfig {
    fn default() -> Self {
        HypervisorConfig {
            kernel_path: "/usr/share/kata-containers/vmlinux.container".into(),
            initrd_path: None,
            image_path: Some("/usr/share/kata-containers/kata-containers.img".into()),
            firmware_path: None,
            kernel_params: "console=hvc0 console=hvc1 reboot=k panic=1".into(),
            default_vcpus: 1,
            default_max_vcpus: 0,
            default_mem_mib: 2048,
            default_max_mem_mib: 0,
            block_device_driver: "virtio-blk".into(),
            disable_nesting_checks: false,
            enable_iommu: false,
        }
    }
}

/// QEMU-specific config — `virtcontainers/qemu.go::QemuConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QemuConfig {
    pub base: HypervisorConfig,
    pub qemu_path: String,
    pub machine_type: String,
    pub virtio_fs_daemon: Option<String>,
}

impl Default for QemuConfig {
    fn default() -> Self {
        QemuConfig {
            base: HypervisorConfig::default(),
            qemu_path: "/usr/bin/qemu-system-x86_64".into(),
            machine_type: "q35".into(),
            virtio_fs_daemon: Some("/usr/libexec/virtiofsd".into()),
        }
    }
}

/// cloud-hypervisor — `virtcontainers/clh.go::CloudHypervisorConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudHypervisorConfig {
    pub base: HypervisorConfig,
    pub clh_path: String,
    pub api_socket: String,
}

impl Default for CloudHypervisorConfig {
    fn default() -> Self {
        CloudHypervisorConfig {
            base: HypervisorConfig::default(),
            clh_path: "/usr/bin/cloud-hypervisor".into(),
            api_socket: "/run/cave-sandbox/clh.sock".into(),
        }
    }
}

/// Firecracker — `virtcontainers/firecracker.go::FirecrackerConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FirecrackerConfig {
    pub base: HypervisorConfig,
    pub fc_path: String,
    pub api_socket: String,
    pub jailer_path: Option<String>,
}

impl Default for FirecrackerConfig {
    fn default() -> Self {
        FirecrackerConfig {
            base: HypervisorConfig::default(),
            fc_path: "/usr/bin/firecracker".into(),
            api_socket: "/run/cave-sandbox/fc.sock".into(),
            jailer_path: Some("/usr/bin/jailer".into()),
        }
    }
}

/// Hypervisor trait — `virtcontainers/hypervisor.go::Hypervisor` interface.
#[async_trait]
pub trait Hypervisor: Send + Sync {
    fn kind(&self) -> HypervisorType;
    async fn create_vm(&self, sandbox_id: &str) -> Result<u32, String>;
    async fn start_vm(&self, sandbox_id: &str) -> Result<(), String>;
    async fn stop_vm(&self, sandbox_id: &str) -> Result<(), String>;
    async fn pause_vm(&self, sandbox_id: &str) -> Result<(), String>;
    async fn resume_vm(&self, sandbox_id: &str) -> Result<(), String>;
}

/// Mock hypervisor — for unit tests and dry-run mode.
pub struct MockHypervisor {
    next_pid: parking_lot::Mutex<u32>,
}

impl Default for MockHypervisor {
    fn default() -> Self {
        MockHypervisor { next_pid: parking_lot::Mutex::new(10_000) }
    }
}

#[async_trait]
impl Hypervisor for MockHypervisor {
    fn kind(&self) -> HypervisorType { HypervisorType::Mock }
    async fn create_vm(&self, _sandbox_id: &str) -> Result<u32, String> {
        let mut g = self.next_pid.lock();
        *g += 1;
        Ok(*g)
    }
    async fn start_vm(&self, _: &str) -> Result<(), String> { Ok(()) }
    async fn stop_vm(&self, _: &str) -> Result<(), String> { Ok(()) }
    async fn pause_vm(&self, _: &str) -> Result<(), String> { Ok(()) }
    async fn resume_vm(&self, _: &str) -> Result<(), String> { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypervisor_kind_strings() {
        assert_eq!(HypervisorType::Qemu.as_str(), "qemu");
        assert_eq!(HypervisorType::Firecracker.as_str(), "firecracker");
        assert_eq!(HypervisorType::CloudHypervisor.as_str(), "cloud-hypervisor");
    }

    #[test]
    fn qemu_default_q35() {
        let q = QemuConfig::default();
        assert_eq!(q.machine_type, "q35");
        assert!(q.virtio_fs_daemon.is_some());
    }

    #[test]
    fn firecracker_default_has_jailer() {
        let f = FirecrackerConfig::default();
        assert!(f.jailer_path.is_some());
        assert_eq!(f.base.block_device_driver, "virtio-blk");
    }

    #[test]
    fn clh_socket_under_cave() {
        let c = CloudHypervisorConfig::default();
        assert!(c.api_socket.contains("cave-sandbox"));
    }

    #[test]
    fn config_roundtrip() {
        let c = HypervisorConfig::default();
        let j = serde_json::to_string(&c).unwrap();
        let back: HypervisorConfig = serde_json::from_str(&j).unwrap();
        assert_eq!(back, c);
    }

    #[tokio::test]
    async fn mock_create_increments() {
        let m = MockHypervisor::default();
        let p1 = m.create_vm("s").await.unwrap();
        let p2 = m.create_vm("s").await.unwrap();
        assert!(p2 > p1);
        assert_eq!(m.kind(), HypervisorType::Mock);
    }

    #[tokio::test]
    async fn mock_lifecycle_ok() {
        let m = MockHypervisor::default();
        m.start_vm("s").await.unwrap();
        m.pause_vm("s").await.unwrap();
        m.resume_vm("s").await.unwrap();
        m.stop_vm("s").await.unwrap();
    }

    #[test]
    fn default_mem_2gib() {
        assert_eq!(HypervisorConfig::default().default_mem_mib, 2048);
    }
}

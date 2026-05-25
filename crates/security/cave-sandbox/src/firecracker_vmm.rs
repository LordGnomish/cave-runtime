// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: VMM resource model adapted from firecracker-microvm/firecracker
// src/vmm/src/resources.rs (Apache-2.0).
//! Firecracker VMM resources — config shape consumed by the REST API.
//!
//! Actual KVM ioctls, vsock kernel module interaction, and tap-iface
//! creation are OUT OF SCOPE (no-FFI, no-CAP_NET_ADMIN policy).

use serde::{Deserialize, Serialize};

/// `PUT /machine-config` body — `MachineConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MachineConfig {
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    #[serde(default)]
    pub smt: bool,
    /// "None" | "T2" | "T2S" | "C3".
    pub cpu_template: Option<String>,
    #[serde(default)]
    pub track_dirty_pages: bool,
}

impl Default for MachineConfig {
    fn default() -> Self {
        MachineConfig { vcpu_count: 1, mem_size_mib: 128, smt: false, cpu_template: None, track_dirty_pages: false }
    }
}

/// `PUT /boot-source`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BootSource {
    pub kernel_image_path: String,
    pub boot_args: Option<String>,
    pub initrd_path: Option<String>,
}

/// `PUT /drives/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Drive {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
    #[serde(default)]
    pub partuuid: Option<String>,
    /// "Sync" | "Async".
    #[serde(default)]
    pub io_engine: Option<String>,
    pub cache_type: Option<String>,
    pub rate_limiter: Option<RateLimiter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RateLimiter {
    pub bandwidth: Option<TokenBucket>,
    pub ops: Option<TokenBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenBucket {
    pub size: u64,
    pub one_time_burst: Option<u64>,
    pub refill_time: u64,
}

/// `PUT /network-interfaces/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NetworkInterface {
    pub iface_id: String,
    pub host_dev_name: String,
    pub guest_mac: Option<String>,
    pub rx_rate_limiter: Option<RateLimiter>,
    pub tx_rate_limiter: Option<RateLimiter>,
}

/// `PUT /vsock`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Vsock {
    pub vsock_id: String,
    pub guest_cid: u32,
    pub uds_path: String,
}

/// `PUT /balloon`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Balloon {
    pub amount_mib: u32,
    pub deflate_on_oom: bool,
    pub stats_polling_interval_s: u32,
}

impl Default for Balloon {
    fn default() -> Self {
        Balloon { amount_mib: 0, deflate_on_oom: true, stats_polling_interval_s: 0 }
    }
}

/// `PUT /logger`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Logger {
    pub log_path: String,
    pub level: Option<String>,
    pub show_level: bool,
    pub show_log_origin: bool,
}

/// `PUT /metrics`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Metrics {
    pub metrics_path: String,
}

/// `PUT /entropy`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Entropy {
    pub rate_limiter: Option<RateLimiter>,
}

/// Aggregate VMM resources — `vmm::resources::VmResources`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct VmResources {
    pub machine_config: MachineConfig,
    pub boot_source: BootSource,
    pub drives: Vec<Drive>,
    pub network_interfaces: Vec<NetworkInterface>,
    pub vsock: Option<Vsock>,
    pub balloon: Option<Balloon>,
    pub logger: Option<Logger>,
    pub metrics: Option<Metrics>,
    pub entropy: Option<Entropy>,
}

impl VmResources {
    /// Validate — same invariants as `validate()` upstream.
    pub fn validate(&self) -> Result<(), String> {
        if self.boot_source.kernel_image_path.is_empty() {
            return Err("kernel_image_path required".into());
        }
        if self.machine_config.vcpu_count == 0 {
            return Err("vcpu_count must be > 0".into());
        }
        if self.machine_config.mem_size_mib == 0 {
            return Err("mem_size_mib must be > 0".into());
        }
        let roots = self.drives.iter().filter(|d| d.is_root_device).count();
        if roots > 1 {
            return Err("more than one root drive".into());
        }
        Ok(())
    }

    /// Mark a drive as root and return mut ref.
    pub fn add_drive(&mut self, d: Drive) {
        self.drives.push(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_default_one_vcpu_128mb() {
        let m = MachineConfig::default();
        assert_eq!(m.vcpu_count, 1);
        assert_eq!(m.mem_size_mib, 128);
    }

    #[test]
    fn validate_empty_kernel_fails() {
        let r = VmResources::default();
        assert!(r.validate().is_err());
    }

    #[test]
    fn validate_ok_minimum() {
        let mut r = VmResources::default();
        r.boot_source.kernel_image_path = "/k/vmlinux".into();
        assert!(r.validate().is_ok());
    }

    #[test]
    fn two_root_drives_invalid() {
        let mut r = VmResources::default();
        r.boot_source.kernel_image_path = "/k".into();
        r.add_drive(Drive {
            drive_id: "rootfs".into(), path_on_host: "/a".into(), is_root_device: true,
            is_read_only: true, partuuid: None, io_engine: None, cache_type: None, rate_limiter: None,
        });
        r.add_drive(Drive {
            drive_id: "rootfs2".into(), path_on_host: "/b".into(), is_root_device: true,
            is_read_only: true, partuuid: None, io_engine: None, cache_type: None, rate_limiter: None,
        });
        assert!(r.validate().is_err());
    }

    #[test]
    fn vsock_serializes_guest_cid() {
        let v = Vsock { vsock_id: "v0".into(), guest_cid: 3, uds_path: "/run/v.sock".into() };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["guest_cid"], 3);
    }

    #[test]
    fn balloon_default_deflate_on_oom() {
        assert!(Balloon::default().deflate_on_oom);
    }

    #[test]
    fn netif_default_no_mac() {
        let n = NetworkInterface::default();
        assert!(n.guest_mac.is_none());
    }

    #[test]
    fn drive_roundtrip() {
        let d = Drive {
            drive_id: "rootfs".into(),
            path_on_host: "/rootfs.ext4".into(),
            is_root_device: true,
            is_read_only: true,
            partuuid: Some("UUID".into()),
            io_engine: Some("Sync".into()),
            cache_type: Some("Unsafe".into()),
            rate_limiter: None,
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: Drive = serde_json::from_str(&j).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn rate_limiter_token_bucket() {
        let rl = RateLimiter {
            bandwidth: Some(TokenBucket { size: 1_000_000, one_time_burst: Some(2_000_000), refill_time: 1000 }),
            ops: None,
        };
        assert!(rl.bandwidth.is_some());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cgroupv2 layout manager.
//!
//! cave-k8s ships cgroupv2-only — the legacy v1 hierarchy and the
//! `kubelet --cgroup-driver=cgroupfs|systemd` branching are not
//! supported.  Mirrors `pkg/kubelet/cm/cgroup_manager_linux.go` of
//! upstream Kubernetes (`v1.32.0`) at the path-shape level.

use serde::{Deserialize, Serialize};

/// Per-pod / per-container cgroupv2 path.  cave-k8s organises Pods
/// under `/sys/fs/cgroup/kubepods.slice/kubepods-<qos>.slice` per the
/// QoS class assignment in `pkg/kubelet/qos`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupPath(pub String);

impl CgroupPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QosClass {
    Guaranteed,
    Burstable,
    BestEffort,
}

impl QosClass {
    pub fn slice(self) -> &'static str {
        match self {
            QosClass::Guaranteed => "kubepods-podsguaranteed.slice",
            QosClass::Burstable => "kubepods-burstable.slice",
            QosClass::BestEffort => "kubepods-besteffort.slice",
        }
    }
}

/// Classify a Pod's QoS class from its container resource shape.  Mirrors
/// `pkg/apis/core/v1/helper/qos/qos.go`.  `requests == limits` (and both > 0)
/// for every container → Guaranteed; any container has a request → Burstable;
/// otherwise → BestEffort.
pub fn classify_qos(containers: &[(u64, u64, u64, u64)]) -> QosClass {
    // tuple = (cpu_req_millis, mem_req_bytes, cpu_lim_millis, mem_lim_bytes)
    if containers.is_empty() {
        return QosClass::BestEffort;
    }
    let mut every_guaranteed = true;
    let mut any_request = false;
    for &(creq, mreq, clim, mlim) in containers {
        if creq > 0 || mreq > 0 || clim > 0 || mlim > 0 {
            any_request = true;
        }
        if !(creq > 0 && mreq > 0 && creq == clim && mreq == mlim) {
            every_guaranteed = false;
        }
    }
    if every_guaranteed {
        QosClass::Guaranteed
    } else if any_request {
        QosClass::Burstable
    } else {
        QosClass::BestEffort
    }
}

pub fn pod_cgroup_path(qos: QosClass, pod_uid: &str) -> CgroupPath {
    CgroupPath(format!(
        "/sys/fs/cgroup/kubepods.slice/{}/kubepods-pod{}.slice",
        qos.slice(),
        pod_uid.replace('-', "_")
    ))
}

pub fn container_cgroup_path(pod: &CgroupPath, container_id: &str) -> CgroupPath {
    CgroupPath(format!("{}/cri-containerd-{}.scope", pod.0, container_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupLimits {
    /// CPU quota in microseconds per 100 ms period; `u32::MAX` ⇒ unlimited.
    pub cpu_quota_us: u32,
    /// Memory limit in bytes; `u64::MAX` ⇒ unlimited.
    pub memory_limit_bytes: u64,
    /// `pids.max`; 0 ⇒ unlimited.
    pub pids_max: u32,
}

impl CgroupLimits {
    pub fn from_pod_resources(
        cpu_limit_millis: u32,
        memory_limit_bytes: u64,
        pids_max: u32,
    ) -> Self {
        let cpu_quota_us = if cpu_limit_millis == 0 {
            u32::MAX
        } else {
            // 100 ms period * cpu_millis/1000 = quota microseconds
            (cpu_limit_millis as u64 * 100_000 / 1000) as u32
        };
        Self {
            cpu_quota_us,
            memory_limit_bytes: if memory_limit_bytes == 0 {
                u64::MAX
            } else {
                memory_limit_bytes
            },
            pids_max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pod_is_besteffort() {
        assert_eq!(classify_qos(&[]), QosClass::BestEffort);
    }

    #[test]
    fn matched_req_lim_is_guaranteed() {
        assert_eq!(
            classify_qos(&[(500, 1024 * 1024, 500, 1024 * 1024)]),
            QosClass::Guaranteed
        );
    }

    #[test]
    fn any_request_no_limit_is_burstable() {
        assert_eq!(
            classify_qos(&[(100, 0, 0, 0)]),
            QosClass::Burstable
        );
    }

    #[test]
    fn no_requests_is_besteffort() {
        assert_eq!(classify_qos(&[(0, 0, 0, 0), (0, 0, 0, 0)]), QosClass::BestEffort);
    }

    #[test]
    fn mismatched_req_lim_is_burstable() {
        assert_eq!(
            classify_qos(&[(500, 1024, 1000, 1024)]),
            QosClass::Burstable
        );
    }

    #[test]
    fn pod_cgroup_path_includes_slice() {
        let p = pod_cgroup_path(QosClass::Burstable, "abc-def");
        assert!(p.as_str().contains("kubepods-burstable.slice"));
        assert!(p.as_str().contains("podabc_def"));
    }

    #[test]
    fn container_cgroup_path_nested() {
        let pod = pod_cgroup_path(QosClass::Guaranteed, "u1");
        let c = container_cgroup_path(&pod, "ctr-1");
        assert!(c.as_str().starts_with(pod.as_str()));
        assert!(c.as_str().ends_with(".scope"));
    }

    #[test]
    fn cgroup_limits_unlimited_sentinels() {
        let l = CgroupLimits::from_pod_resources(0, 0, 0);
        assert_eq!(l.cpu_quota_us, u32::MAX);
        assert_eq!(l.memory_limit_bytes, u64::MAX);
        assert_eq!(l.pids_max, 0);
    }

    #[test]
    fn cgroup_limits_compute_cpu_quota() {
        let l = CgroupLimits::from_pod_resources(500, 1024, 256);
        // 500m * 100ms = 50ms = 50_000us
        assert_eq!(l.cpu_quota_us, 50_000);
        assert_eq!(l.memory_limit_bytes, 1024);
        assert_eq!(l.pids_max, 256);
    }
}

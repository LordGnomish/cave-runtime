//! cgroup manager — hierarchy and resource-limit translation for v1 + v2.
//!
//! Mirrors `pkg/kubelet/cm/cgroup_manager_*.go`: builds the kubelet's
//! cgroup hierarchy (`kubepods.slice` → QoS subgroup → pod → container),
//! translates Pod resource specs into the on-disk fields the kernel
//! expects, computes per-pod OOM score adjustments, and produces v1- vs
//! v2-flavored writeable file maps.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CgroupVersion {
    V1,
    V2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CgroupDriver {
    /// `cgroupfs` driver writes paths directly under `/sys/fs/cgroup`.
    Cgroupfs,
    /// `systemd` driver delegates to systemd via slice/scope units.
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QosClass {
    Guaranteed,
    Burstable,
    BestEffort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupResources {
    /// CPU request in millicores.
    pub cpu_request_milli: u64,
    /// CPU limit in millicores; `0` = unlimited.
    pub cpu_limit_milli: u64,
    /// Memory limit in bytes; `0` = unlimited.
    pub memory_limit_bytes: u64,
    /// HugePages 2Mi reservation in bytes.
    pub hugepages_2mi: u64,
    /// HugePages 1Gi reservation in bytes.
    pub hugepages_1gi: u64,
    /// Swap limit in bytes (cgroup v2 only).
    pub swap_limit_bytes: Option<u64>,
    /// Whether `cpu.idle = 1` should be set (KEP-2570 SCHED_IDLE).
    pub cpu_idle: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CgroupError {
    #[error("invalid: {0}")]
    Invalid(String),
}

pub type CgroupResult<T> = Result<T, CgroupError>;

/// Compute the cgroup path for a pod given the driver and the QoS class.
/// Driver=cgroupfs uses slash-paths; driver=systemd uses dash-encoded slices.
pub fn pod_cgroup_path(
    driver: CgroupDriver,
    cgroup_root: &str,
    qos: QosClass,
    pod_uid: &str,
) -> String {
    match driver {
        CgroupDriver::Cgroupfs => {
            let qos_seg = qos_segment_cgroupfs(qos);
            if qos_seg.is_empty() {
                format!("{}/pod{}", cgroup_root.trim_end_matches('/'), pod_uid)
            } else {
                format!(
                    "{}/{}/pod{}",
                    cgroup_root.trim_end_matches('/'),
                    qos_seg,
                    pod_uid
                )
            }
        }
        CgroupDriver::Systemd => {
            let prefix = cgroup_root.trim_end_matches('/').trim_start_matches('/');
            let qos_seg = qos_segment_systemd(qos);
            // Systemd hides slashes by encoding them as dashes; pod UID dashes → underscores.
            let pod_seg = format!("kubepods-pod{}.slice", pod_uid.replace('-', "_"));
            if qos_seg.is_empty() {
                format!("/{}.slice/{}", prefix, pod_seg)
            } else {
                format!("/{}.slice/{}.slice/{}", prefix, qos_seg, pod_seg)
            }
        }
    }
}

fn qos_segment_cgroupfs(qos: QosClass) -> &'static str {
    match qos {
        QosClass::Guaranteed => "",
        QosClass::Burstable => "burstable",
        QosClass::BestEffort => "besteffort",
    }
}

fn qos_segment_systemd(qos: QosClass) -> &'static str {
    match qos {
        QosClass::Guaranteed => "",
        QosClass::Burstable => "kubepods-burstable",
        QosClass::BestEffort => "kubepods-besteffort",
    }
}

/// Container cgroup path under a pod cgroup. v1 produces a per-container dir;
/// v2 may share the pod path with cgroup namespaces.
pub fn container_cgroup_path(pod_path: &str, container_id: &str) -> String {
    format!("{}/{}", pod_path, container_id)
}

/// Translate `cpu_request_milli` → cpu.shares (v1) using upstream's formula:
/// `shares = (millicores * 1024) / 1000`, with floor at 2.
pub fn cpu_shares_v1(milli: u64) -> u64 {
    if milli == 0 {
        return 2;
    }
    let shares = (milli as u128 * 1024) / 1000;
    shares.max(2) as u64
}

/// Translate `cpu_request_milli` → cpu.weight (v2). Linear map of cpu.shares
/// (range 2..=262144) onto cpu.weight (1..=10000) per upstream's
/// `convertCPUSharesToCgroupV2Weight`.
pub fn cpu_weight_v2(milli: u64) -> u64 {
    let shares = cpu_shares_v1(milli);
    // shares 2 → weight 1; shares 262144 → weight 10000.
    let w = 1 + ((shares.saturating_sub(2) * 9999) / (262144 - 2));
    w.clamp(1, 10000)
}

/// Translate `cpu_limit_milli` to (cpu.cfs_quota_us, cpu.cfs_period_us) for v1.
/// Period default is 100 000 µs (100 ms). Quota = period * (milli / 1000).
/// `cpu_limit_milli == 0` means unlimited → quota = -1.
pub fn cpu_cfs_quota_period_v1(milli: u64) -> (i64, u64) {
    let period: u64 = 100_000;
    if milli == 0 {
        return (-1, period);
    }
    let quota = (milli as i64 * period as i64) / 1000;
    (quota.max(1000), period)
}

/// v2 form: `cpu.max = "<quota> <period>"`. Unlimited → "max <period>".
pub fn cpu_max_v2(milli: u64) -> String {
    let period: u64 = 100_000;
    if milli == 0 {
        return format!("max {}", period);
    }
    let quota = (milli as u64 * period) / 1000;
    format!("{} {}", quota.max(1000), period)
}

/// memory.limit_in_bytes (v1) / memory.max (v2). 0 → unlimited.
pub fn memory_limit_v1(bytes: u64) -> i64 {
    if bytes == 0 {
        return -1;
    }
    bytes as i64
}

pub fn memory_max_v2(bytes: u64) -> String {
    if bytes == 0 {
        "max".into()
    } else {
        bytes.to_string()
    }
}

/// memory.swap.max (v2 only). Default to 0 (no swap allowed) unless explicit.
pub fn memory_swap_max_v2(swap_bytes: Option<u64>) -> String {
    match swap_bytes {
        None => "0".into(),
        Some(0) => "0".into(),
        Some(b) => b.to_string(),
    }
}

/// Compute oom_score_adj per upstream's `pkg/kubelet/qos/policy.go`:
///   - Guaranteed: -997
///   - Burstable: clamped to [2, 999] based on (1000 - request%) heuristic
///   - BestEffort: 1000
pub fn oom_score_adj(qos: QosClass, memory_request_bytes: u64, node_memory_bytes: u64) -> i32 {
    match qos {
        QosClass::Guaranteed => -997,
        QosClass::BestEffort => 1000,
        QosClass::Burstable => {
            if node_memory_bytes == 0 {
                return 999;
            }
            // Pod with higher memory request gets lower score.
            // upstream formula: max(2, min(1000 - 1000*req/total, 999))
            let pct = (memory_request_bytes * 1000) / node_memory_bytes.max(1);
            let v = 1000_i64.saturating_sub(pct as i64);
            v.clamp(2, 999) as i32
        }
    }
}

/// Compute writeable file map for a container's cgroup, given version.
pub fn writeable_files(
    version: CgroupVersion,
    res: &CgroupResources,
) -> CgroupResult<BTreeMap<String, String>> {
    if res.cpu_request_milli > 0 && res.cpu_limit_milli > 0
        && res.cpu_request_milli > res.cpu_limit_milli
    {
        return Err(CgroupError::Invalid(
            "cpu request greater than limit".into(),
        ));
    }
    let mut out = BTreeMap::new();
    match version {
        CgroupVersion::V1 => {
            out.insert(
                "cpu.shares".into(),
                cpu_shares_v1(res.cpu_request_milli).to_string(),
            );
            let (quota, period) = cpu_cfs_quota_period_v1(res.cpu_limit_milli);
            out.insert("cpu.cfs_quota_us".into(), quota.to_string());
            out.insert("cpu.cfs_period_us".into(), period.to_string());
            out.insert(
                "memory.limit_in_bytes".into(),
                memory_limit_v1(res.memory_limit_bytes).to_string(),
            );
            if res.hugepages_2mi > 0 {
                out.insert(
                    "hugetlb.2MB.limit_in_bytes".into(),
                    res.hugepages_2mi.to_string(),
                );
            }
            if res.hugepages_1gi > 0 {
                out.insert(
                    "hugetlb.1GB.limit_in_bytes".into(),
                    res.hugepages_1gi.to_string(),
                );
            }
        }
        CgroupVersion::V2 => {
            out.insert(
                "cpu.weight".into(),
                cpu_weight_v2(res.cpu_request_milli).to_string(),
            );
            out.insert("cpu.max".into(), cpu_max_v2(res.cpu_limit_milli));
            out.insert("memory.max".into(), memory_max_v2(res.memory_limit_bytes));
            out.insert(
                "memory.swap.max".into(),
                memory_swap_max_v2(res.swap_limit_bytes),
            );
            if res.cpu_idle {
                out.insert("cpu.idle".into(), "1".into());
            }
            if res.hugepages_2mi > 0 {
                out.insert(
                    "hugetlb.2MB.max".into(),
                    res.hugepages_2mi.to_string(),
                );
            }
            if res.hugepages_1gi > 0 {
                out.insert(
                    "hugetlb.1GB.max".into(),
                    res.hugepages_1gi.to_string(),
                );
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res() -> CgroupResources {
        CgroupResources {
            cpu_request_milli: 100,
            cpu_limit_milli: 200,
            memory_limit_bytes: 1024 * 1024 * 128,
            hugepages_2mi: 0,
            hugepages_1gi: 0,
            swap_limit_bytes: None,
            cpu_idle: false,
        }
    }

    #[test]
    fn pod_path_cgroupfs_guaranteed_skips_qos_segment() {
        let p = pod_cgroup_path(
            CgroupDriver::Cgroupfs,
            "/sys/fs/cgroup/kubepods",
            QosClass::Guaranteed,
            "abc123",
        );
        assert_eq!(p, "/sys/fs/cgroup/kubepods/podabc123");
    }

    #[test]
    fn pod_path_cgroupfs_burstable_includes_qos() {
        let p = pod_cgroup_path(
            CgroupDriver::Cgroupfs,
            "/sys/fs/cgroup/kubepods",
            QosClass::Burstable,
            "abc",
        );
        assert!(p.contains("/burstable/"));
    }

    #[test]
    fn pod_path_cgroupfs_besteffort_includes_qos() {
        let p = pod_cgroup_path(
            CgroupDriver::Cgroupfs,
            "/sys/fs/cgroup/kubepods",
            QosClass::BestEffort,
            "abc",
        );
        assert!(p.contains("/besteffort/"));
    }

    #[test]
    fn pod_path_systemd_uses_slice_units() {
        let p = pod_cgroup_path(
            CgroupDriver::Systemd,
            "kubepods",
            QosClass::Burstable,
            "abc-def",
        );
        assert!(p.contains(".slice"));
        assert!(p.contains("kubepods-burstable.slice"));
        assert!(p.contains("podabc_def"));
    }

    #[test]
    fn pod_path_systemd_guaranteed_no_qos_subslice() {
        let p = pod_cgroup_path(
            CgroupDriver::Systemd,
            "kubepods",
            QosClass::Guaranteed,
            "abc",
        );
        assert!(p.contains(".slice"));
        assert!(!p.contains("burstable"));
        assert!(!p.contains("besteffort"));
    }

    #[test]
    fn container_path_appends_id() {
        let p = container_cgroup_path("/sys/fs/cgroup/kubepods/podabc", "ctr-1");
        assert_eq!(p, "/sys/fs/cgroup/kubepods/podabc/ctr-1");
    }

    #[test]
    fn cpu_shares_v1_formula() {
        assert_eq!(cpu_shares_v1(0), 2);
        assert_eq!(cpu_shares_v1(1), 2); // floor at 2
        assert_eq!(cpu_shares_v1(1000), 1024);
        assert_eq!(cpu_shares_v1(2000), 2048);
    }

    #[test]
    fn cpu_weight_v2_min_max_bounds() {
        assert_eq!(cpu_weight_v2(0), 1);
        // 1 core ≈ shares 1024 ≈ middle weight; verify bounds.
        let w = cpu_weight_v2(1000);
        assert!(w >= 1 && w <= 10000);
        // Very high should saturate.
        assert_eq!(cpu_weight_v2(10_000_000), 10000);
    }

    #[test]
    fn cpu_cfs_quota_period_v1_unlimited() {
        let (q, p) = cpu_cfs_quota_period_v1(0);
        assert_eq!(q, -1);
        assert_eq!(p, 100_000);
    }

    #[test]
    fn cpu_cfs_quota_period_v1_2_cores() {
        let (q, _) = cpu_cfs_quota_period_v1(2000);
        assert_eq!(q, 200_000);
    }

    #[test]
    fn cpu_cfs_quota_floor_1ms() {
        let (q, _) = cpu_cfs_quota_period_v1(1);
        assert_eq!(q, 1000);
    }

    #[test]
    fn cpu_max_v2_unlimited_format() {
        assert_eq!(cpu_max_v2(0), "max 100000");
    }

    #[test]
    fn cpu_max_v2_specific_cores() {
        assert_eq!(cpu_max_v2(1500), "150000 100000");
    }

    #[test]
    fn memory_limit_v1_unlimited_negative_one() {
        assert_eq!(memory_limit_v1(0), -1);
        assert_eq!(memory_limit_v1(1024), 1024);
    }

    #[test]
    fn memory_max_v2_format() {
        assert_eq!(memory_max_v2(0), "max");
        assert_eq!(memory_max_v2(1024), "1024");
    }

    #[test]
    fn memory_swap_max_v2_default_zero() {
        assert_eq!(memory_swap_max_v2(None), "0");
        assert_eq!(memory_swap_max_v2(Some(0)), "0");
        assert_eq!(memory_swap_max_v2(Some(2048)), "2048");
    }

    #[test]
    fn oom_score_guaranteed_is_neg_997() {
        assert_eq!(oom_score_adj(QosClass::Guaranteed, 1024, 1024_000), -997);
    }

    #[test]
    fn oom_score_besteffort_is_1000() {
        assert_eq!(oom_score_adj(QosClass::BestEffort, 0, 1024_000), 1000);
    }

    #[test]
    fn oom_score_burstable_clamped_to_2_999() {
        let s = oom_score_adj(QosClass::Burstable, 0, 1024);
        assert!(s >= 2 && s <= 999);
    }

    #[test]
    fn oom_score_burstable_zero_node_memory_returns_999() {
        assert_eq!(oom_score_adj(QosClass::Burstable, 1, 0), 999);
    }

    #[test]
    fn oom_score_burstable_higher_request_lower_score() {
        let low_req = oom_score_adj(QosClass::Burstable, 100, 1_000_000);
        let high_req = oom_score_adj(QosClass::Burstable, 500_000, 1_000_000);
        assert!(high_req < low_req);
    }

    #[test]
    fn writeable_files_v1_includes_shares_quota_period_memory() {
        let f = writeable_files(CgroupVersion::V1, &res()).unwrap();
        assert!(f.contains_key("cpu.shares"));
        assert!(f.contains_key("cpu.cfs_quota_us"));
        assert!(f.contains_key("cpu.cfs_period_us"));
        assert!(f.contains_key("memory.limit_in_bytes"));
    }

    #[test]
    fn writeable_files_v2_includes_weight_max_memory_swap() {
        let f = writeable_files(CgroupVersion::V2, &res()).unwrap();
        assert!(f.contains_key("cpu.weight"));
        assert!(f.contains_key("cpu.max"));
        assert!(f.contains_key("memory.max"));
        assert!(f.contains_key("memory.swap.max"));
    }

    #[test]
    fn writeable_files_v2_swap_default_zero() {
        let f = writeable_files(CgroupVersion::V2, &res()).unwrap();
        assert_eq!(f.get("memory.swap.max"), Some(&"0".to_string()));
    }

    #[test]
    fn writeable_files_v2_swap_explicit() {
        let mut r = res();
        r.swap_limit_bytes = Some(4096);
        let f = writeable_files(CgroupVersion::V2, &r).unwrap();
        assert_eq!(f.get("memory.swap.max"), Some(&"4096".to_string()));
    }

    #[test]
    fn writeable_files_v2_cpu_idle_when_set() {
        let mut r = res();
        r.cpu_idle = true;
        let f = writeable_files(CgroupVersion::V2, &r).unwrap();
        assert_eq!(f.get("cpu.idle"), Some(&"1".to_string()));
    }

    #[test]
    fn writeable_files_v2_cpu_idle_absent_when_unset() {
        let r = res();
        let f = writeable_files(CgroupVersion::V2, &r).unwrap();
        assert!(!f.contains_key("cpu.idle"));
    }

    #[test]
    fn writeable_files_hugepages_v1() {
        let mut r = res();
        r.hugepages_2mi = 2 * 1024 * 1024 * 1024;
        let f = writeable_files(CgroupVersion::V1, &r).unwrap();
        assert!(f.contains_key("hugetlb.2MB.limit_in_bytes"));
    }

    #[test]
    fn writeable_files_hugepages_v2() {
        let mut r = res();
        r.hugepages_1gi = 1024 * 1024 * 1024;
        let f = writeable_files(CgroupVersion::V2, &r).unwrap();
        assert!(f.contains_key("hugetlb.1GB.max"));
    }

    #[test]
    fn writeable_files_rejects_request_greater_than_limit() {
        let mut r = res();
        r.cpu_request_milli = 500;
        r.cpu_limit_milli = 200;
        assert!(writeable_files(CgroupVersion::V1, &r).is_err());
    }

    #[test]
    fn writeable_files_no_limit_no_request_ok() {
        let mut r = res();
        r.cpu_request_milli = 0;
        r.cpu_limit_milli = 0;
        r.memory_limit_bytes = 0;
        let f = writeable_files(CgroupVersion::V1, &r).unwrap();
        assert_eq!(f.get("cpu.cfs_quota_us"), Some(&"-1".to_string()));
        assert_eq!(f.get("memory.limit_in_bytes"), Some(&"-1".to_string()));
    }

    #[test]
    fn writeable_files_v2_unlimited_form_serialised() {
        let mut r = res();
        r.cpu_limit_milli = 0;
        r.memory_limit_bytes = 0;
        let f = writeable_files(CgroupVersion::V2, &r).unwrap();
        assert_eq!(f.get("cpu.max"), Some(&"max 100000".to_string()));
        assert_eq!(f.get("memory.max"), Some(&"max".to_string()));
    }
}

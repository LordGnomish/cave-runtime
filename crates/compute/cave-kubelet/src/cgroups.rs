// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cgroup v2 unified-hierarchy writer abstraction.
//!
//! Mirrors `pkg/kubelet/cm/util/cgroups/` from upstream's
//! systemd-cgroup-driver path. The CRI runtime already
//! translates pod resource requests into cgroup writes via
//! containerd / CRI-O; this module gives the kubelet a direct
//! cgroup-write surface for the cases the spec defines (KEP-2832
//! memory.max for OOM-kill prevention, KEP-3650 cpuset for QoS
//! tier isolation).
//!
//! ## Scope
//!
//! * **In-memory abstraction** — `CgroupBackend` trait + a test
//!   `InMemoryCgroups` impl. The production
//!   `Cgroupv2FsBackend` would write to `/sys/fs/cgroup/` and is
//!   not landed here (real filesystem I/O is the deferred half).
//! * **v2 unified hierarchy only** — cgroup v1 is upstream
//!   `legacy` and out of scope.
//!
//! Path scheme follows upstream: `<root>/kubepods.slice/
//! kubepods-<qos>.slice/kubepods-<qos>-pod<podUID>.slice` for
//! each pod, with one container directory under that.

use std::collections::BTreeMap;
use std::sync::RwLock;

/// Pod QoS tier — drives the slice naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QosTier {
    /// `requests == limits` for every resource.
    Guaranteed,
    /// Some resource has `requests < limits`.
    Burstable,
    /// No `requests` or `limits` set.
    BestEffort,
}

impl QosTier {
    pub fn slice_suffix(self) -> &'static str {
        match self {
            QosTier::Guaranteed => "",
            QosTier::Burstable => "burstable",
            QosTier::BestEffort => "besteffort",
        }
    }
}

/// Resource value the kubelet writes — one of the cgroup v2
/// control files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupValue {
    /// `memory.max` — `"max"` or a byte count.
    MemoryMax(MemoryLimit),
    /// `cpu.max` — `quota period` or `max period`.
    CpuMax { quota: Option<u64>, period_us: u64 },
    /// `cpuset.cpus` / `cpuset.mems` — CSV cpu list.
    CpusetCpus(String),
    /// `pids.max` — process count cap.
    PidsMax(Option<u64>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLimit {
    Bytes(u64),
    Unlimited,
}

/// Fully-qualified cgroup path the kubelet wants to read or
/// write. Built via `pod_cgroup_path` / `container_cgroup_path`
/// to ensure consistent naming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupPath(pub String);

/// Compute the v2 slice path for a pod.
pub fn pod_cgroup_path(qos: QosTier, pod_uid: &str) -> CgroupPath {
    let suffix = qos.slice_suffix();
    let qos_slice = if suffix.is_empty() {
        "kubepods.slice".to_string()
    } else {
        format!("kubepods-{suffix}.slice")
    };
    let pod_slice = if suffix.is_empty() {
        format!("kubepods-pod{pod_uid}.slice")
    } else {
        format!("kubepods-{suffix}-pod{pod_uid}.slice")
    };
    CgroupPath(format!("/{qos_slice}/{pod_slice}"))
}

/// Compute the container scope under a pod slice. cri-runtimes
/// use `cri-containerd-<containerID>.scope`; we mirror that.
pub fn container_cgroup_path(pod_path: &CgroupPath, container_id: &str) -> CgroupPath {
    CgroupPath(format!(
        "{}/cri-containerd-{container_id}.scope",
        pod_path.0
    ))
}

/// Backend trait — the kubelet writes through this. Production
/// installs use `Cgroupv2FsBackend` (writes to `/sys/fs/cgroup`);
/// tests use `InMemoryCgroups`.
pub trait CgroupBackend: Send + Sync {
    fn write(&self, path: &CgroupPath, value: CgroupValue) -> Result<(), CgroupError>;
    fn read(&self, path: &CgroupPath, control: &str) -> Result<Option<CgroupValue>, CgroupError>;
    /// Remove a cgroup entirely (used on pod teardown).
    fn remove(&self, path: &CgroupPath) -> Result<(), CgroupError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupError {
    /// Path doesn't exist and isn't auto-creatable.
    NotFound(String),
    /// Validation failure (e.g. negative quota, malformed cpuset).
    Invalid(String),
    /// Other I/O failure (in the real backend; never raised by
    /// the in-memory impl).
    Io(String),
}

impl std::fmt::Display for CgroupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CgroupError::NotFound(p) => write!(f, "cgroup not found: {p}"),
            CgroupError::Invalid(m) => write!(f, "invalid cgroup value: {m}"),
            CgroupError::Io(m) => write!(f, "cgroup io error: {m}"),
        }
    }
}

impl std::error::Error for CgroupError {}

/// In-memory backend — every `write` is stored in a map keyed
/// by `(path, control-file)`. Used by tests and to exercise
/// the surface without touching `/sys/fs/cgroup`.
#[derive(Default)]
pub struct InMemoryCgroups {
    inner: RwLock<BTreeMap<(String, String), CgroupValue>>,
}

impl InMemoryCgroups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.read().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().expect("poisoned").is_empty()
    }
}

fn control_file(v: &CgroupValue) -> &'static str {
    match v {
        CgroupValue::MemoryMax(_) => "memory.max",
        CgroupValue::CpuMax { .. } => "cpu.max",
        CgroupValue::CpusetCpus(_) => "cpuset.cpus",
        CgroupValue::PidsMax(_) => "pids.max",
    }
}

impl CgroupBackend for InMemoryCgroups {
    fn write(&self, path: &CgroupPath, value: CgroupValue) -> Result<(), CgroupError> {
        if let CgroupValue::CpuMax { quota: Some(q), .. } = &value {
            if *q == 0 {
                return Err(CgroupError::Invalid("cpu quota must not be zero".into()));
            }
        }
        if let CgroupValue::CpusetCpus(s) = &value {
            if s.is_empty() {
                return Err(CgroupError::Invalid("cpuset.cpus must not be empty".into()));
            }
        }
        let cf = control_file(&value).to_string();
        let mut g = self.inner.write().expect("poisoned");
        g.insert((path.0.clone(), cf), value);
        Ok(())
    }

    fn read(&self, path: &CgroupPath, control: &str) -> Result<Option<CgroupValue>, CgroupError> {
        let g = self.inner.read().expect("poisoned");
        Ok(g.get(&(path.0.clone(), control.to_string())).cloned())
    }

    fn remove(&self, path: &CgroupPath) -> Result<(), CgroupError> {
        let mut g = self.inner.write().expect("poisoned");
        g.retain(|(p, _), _| p != &path.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_path_guaranteed_uses_top_slice() {
        let p = pod_cgroup_path(QosTier::Guaranteed, "abc-123");
        assert_eq!(p.0, "/kubepods.slice/kubepods-podabc-123.slice");
    }

    #[test]
    fn pod_path_burstable_uses_qos_subslice() {
        let p = pod_cgroup_path(QosTier::Burstable, "abc-123");
        assert_eq!(
            p.0,
            "/kubepods-burstable.slice/kubepods-burstable-podabc-123.slice"
        );
    }

    #[test]
    fn pod_path_besteffort_uses_qos_subslice() {
        let p = pod_cgroup_path(QosTier::BestEffort, "abc-123");
        assert!(p.0.contains("besteffort"));
    }

    #[test]
    fn container_path_appends_scope() {
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        let c = container_cgroup_path(&p, "ctrid");
        assert!(c.0.ends_with("/cri-containerd-ctrid.scope"));
    }

    #[test]
    fn write_then_read_memory_max() {
        let b = InMemoryCgroups::new();
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        b.write(&p, CgroupValue::MemoryMax(MemoryLimit::Bytes(1_000_000)))
            .unwrap();
        let got = b.read(&p, "memory.max").unwrap();
        assert_eq!(
            got,
            Some(CgroupValue::MemoryMax(MemoryLimit::Bytes(1_000_000)))
        );
    }

    #[test]
    fn write_cpu_max_rejects_zero_quota() {
        let b = InMemoryCgroups::new();
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        let err = b
            .write(
                &p,
                CgroupValue::CpuMax {
                    quota: Some(0),
                    period_us: 100_000,
                },
            )
            .unwrap_err();
        assert!(matches!(err, CgroupError::Invalid(_)));
    }

    #[test]
    fn write_cpuset_rejects_empty() {
        let b = InMemoryCgroups::new();
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        assert!(b.write(&p, CgroupValue::CpusetCpus(String::new())).is_err());
    }

    #[test]
    fn write_pids_max_accepts_unlimited() {
        let b = InMemoryCgroups::new();
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        b.write(&p, CgroupValue::PidsMax(None)).unwrap();
        let got = b.read(&p, "pids.max").unwrap();
        assert_eq!(got, Some(CgroupValue::PidsMax(None)));
    }

    #[test]
    fn read_missing_returns_none() {
        let b = InMemoryCgroups::new();
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        assert_eq!(b.read(&p, "memory.max").unwrap(), None);
    }

    #[test]
    fn remove_drops_every_control_at_path() {
        let b = InMemoryCgroups::new();
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        b.write(&p, CgroupValue::MemoryMax(MemoryLimit::Unlimited))
            .unwrap();
        b.write(&p, CgroupValue::PidsMax(Some(200))).unwrap();
        assert_eq!(b.len(), 2);
        b.remove(&p).unwrap();
        assert!(b.is_empty());
    }

    #[test]
    fn remove_only_drops_matching_path() {
        let b = InMemoryCgroups::new();
        let p1 = pod_cgroup_path(QosTier::Guaranteed, "u1");
        let p2 = pod_cgroup_path(QosTier::Burstable, "u2");
        b.write(&p1, CgroupValue::MemoryMax(MemoryLimit::Bytes(1)))
            .unwrap();
        b.write(&p2, CgroupValue::MemoryMax(MemoryLimit::Bytes(2)))
            .unwrap();
        b.remove(&p1).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(
            b.read(&p2, "memory.max").unwrap(),
            Some(CgroupValue::MemoryMax(MemoryLimit::Bytes(2)))
        );
    }

    #[test]
    fn write_overwrites_previous_value_for_same_control() {
        let b = InMemoryCgroups::new();
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        b.write(&p, CgroupValue::MemoryMax(MemoryLimit::Bytes(1)))
            .unwrap();
        b.write(&p, CgroupValue::MemoryMax(MemoryLimit::Bytes(2)))
            .unwrap();
        let got = b.read(&p, "memory.max").unwrap();
        assert_eq!(got, Some(CgroupValue::MemoryMax(MemoryLimit::Bytes(2))));
    }

    // ── cgroup v2 on-disk serialization (libcontainer/cgroups/fs2) ────────────

    #[test]
    fn serialize_memory_max_bytes_is_decimal() {
        let (cf, content) = serialize_value(&CgroupValue::MemoryMax(MemoryLimit::Bytes(1_048_576)));
        assert_eq!(cf, "memory.max");
        assert_eq!(content, "1048576");
    }

    #[test]
    fn serialize_memory_max_unlimited_is_literal_max() {
        let (cf, content) = serialize_value(&CgroupValue::MemoryMax(MemoryLimit::Unlimited));
        assert_eq!(cf, "memory.max");
        assert_eq!(content, "max");
    }

    #[test]
    fn serialize_cpu_max_quota_period_space_separated() {
        let (cf, content) = serialize_value(&CgroupValue::CpuMax {
            quota: Some(50_000),
            period_us: 100_000,
        });
        assert_eq!(cf, "cpu.max");
        // cgroup v2 cpu.max format is "$QUOTA $PERIOD".
        assert_eq!(content, "50000 100000");
    }

    #[test]
    fn serialize_cpu_max_unlimited_quota_is_max_period() {
        let (_, content) = serialize_value(&CgroupValue::CpuMax {
            quota: None,
            period_us: 100_000,
        });
        assert_eq!(content, "max 100000");
    }

    #[test]
    fn serialize_pids_max_unlimited_is_literal_max() {
        let (cf, content) = serialize_value(&CgroupValue::PidsMax(None));
        assert_eq!(cf, "pids.max");
        assert_eq!(content, "max");
    }

    #[test]
    fn parse_cpu_max_roundtrips() {
        let v = parse_value(
            "cpu.max",
            "50000 100000",
        )
        .unwrap();
        assert_eq!(
            v,
            CgroupValue::CpuMax {
                quota: Some(50_000),
                period_us: 100_000
            }
        );
    }

    #[test]
    fn parse_memory_max_max_is_unlimited() {
        let v = parse_value("memory.max", "max\n").unwrap();
        assert_eq!(v, CgroupValue::MemoryMax(MemoryLimit::Unlimited));
    }

    #[test]
    fn parse_unknown_control_is_invalid() {
        assert!(matches!(
            parse_value("io.weight", "100"),
            Err(CgroupError::Invalid(_))
        ));
    }

    // ── Cgroupv2FsBackend — real filesystem writer rooted at a test dir ──────

    fn temp_root(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!(
            "cave-cg-{}-{}-{}",
            std::process::id(),
            tag,
            n
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn fs_write_creates_nested_slice_dirs_and_control_file() {
        let root = temp_root("nested");
        let b = Cgroupv2FsBackend::new(&root);
        let p = pod_cgroup_path(QosTier::Burstable, "abc-123");
        b.write(&p, CgroupValue::MemoryMax(MemoryLimit::Bytes(2_000_000)))
            .unwrap();
        // The slice hierarchy is materialised on disk, with the control
        // file holding the v2 wire content.
        let dir = root.join(p.0.trim_start_matches('/'));
        assert!(dir.is_dir(), "slice dir should exist: {dir:?}");
        let raw = std::fs::read_to_string(dir.join("memory.max")).unwrap();
        assert_eq!(raw, "2000000");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fs_write_then_read_roundtrips_cpu_max() {
        let root = temp_root("cpu");
        let b = Cgroupv2FsBackend::new(&root);
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        let v = CgroupValue::CpuMax {
            quota: Some(80_000),
            period_us: 100_000,
        };
        b.write(&p, v.clone()).unwrap();
        assert_eq!(b.read(&p, "cpu.max").unwrap(), Some(v));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fs_read_missing_returns_none() {
        let root = temp_root("missing");
        let b = Cgroupv2FsBackend::new(&root);
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        assert_eq!(b.read(&p, "memory.max").unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fs_remove_drops_directory_tree() {
        let root = temp_root("remove");
        let b = Cgroupv2FsBackend::new(&root);
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        b.write(&p, CgroupValue::PidsMax(Some(200))).unwrap();
        let dir = root.join(p.0.trim_start_matches('/'));
        assert!(dir.is_dir());
        b.remove(&p).unwrap();
        assert!(!dir.exists());
        // Removing an absent cgroup is idempotent.
        b.remove(&p).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fs_write_rejects_zero_quota_before_touching_disk() {
        let root = temp_root("zeroquota");
        let b = Cgroupv2FsBackend::new(&root);
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        let err = b
            .write(
                &p,
                CgroupValue::CpuMax {
                    quota: Some(0),
                    period_us: 100_000,
                },
            )
            .unwrap_err();
        assert!(matches!(err, CgroupError::Invalid(_)));
        // Validation must short-circuit — no dir created.
        assert!(!root.join(p.0.trim_start_matches('/')).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fs_write_rejects_empty_cpuset() {
        let root = temp_root("emptycpuset");
        let b = Cgroupv2FsBackend::new(&root);
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        assert!(b.write(&p, CgroupValue::CpusetCpus(String::new())).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fs_read_memory_max_unlimited_parses_back() {
        let root = temp_root("memunlim");
        let b = Cgroupv2FsBackend::new(&root);
        let p = pod_cgroup_path(QosTier::Guaranteed, "u");
        b.write(&p, CgroupValue::MemoryMax(MemoryLimit::Unlimited))
            .unwrap();
        assert_eq!(
            b.read(&p, "memory.max").unwrap(),
            Some(CgroupValue::MemoryMax(MemoryLimit::Unlimited))
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

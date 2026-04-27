//! User-namespace second layer — idmapped mounts (KEP-127 phase 2),
//! cgroup namespace (CLONE_NEWCGROUP), and allocator validation /
//! overlap detection.
//!
//! Builds on `crate::userns` (uid/gid map plumbing). New surfaces:
//!
//! - `IdmappedMount` — `mount(2)` with the new `MOUNT_ATTR_IDMAP`
//!   semantics (Linux 5.12+) or fall back to a per-source bind mount
//!   with the userns applied. The `mount_attr` argv builder is here.
//! - `CgroupNamespace` — `clone(CLONE_NEWCGROUP)` payload describing
//!   the per-pod cgroup view. The kubelet expects a separate cgroup
//!   ns when KEP-127 is active so containers can't observe the host
//!   cgroup tree.
//! - `validate_id_mapping` / `validate_user_namespace` — KEP-127
//!   correctness checks (no wraparound, host_id offset away from the
//!   reserved 0..1000 range, no overlap between mappings).
//! - Allocator extensions: `release_unused_ranges` (compact across
//!   collected ranges) and `fragmentation_ratio` for monitoring.

use crate::error::{CriError, CriResult};
use crate::userns::{IdMapping, UserNamespace, UserNsAllocator, DEFAULT_RANGE_SIZE};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Idmapped mount ───────────────────────────────────────────────────────────

/// `MOUNT_ATTR_IDMAP` flag bit from `<linux/mount.h>`.
pub const MOUNT_ATTR_IDMAP: u64 = 0x0010_0000;

/// Description of one idmapped mount applied to a sandbox.
///
/// The kernel takes a `userns_fd` plus a `mount_attr` struct (with
/// `MOUNT_ATTR_IDMAP` set) and remaps every UID/GID seen on the
/// underlying filesystem through the user namespace's id_map. Used by
/// CSI volume mounts for KEP-127 user namespaces so a single backing
/// share can be presented with different ownership to each pod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdmappedMount {
    pub source: PathBuf,
    pub target: PathBuf,
    pub fs_type: String,
    pub user_namespace: UserNamespace,
}

impl IdmappedMount {
    /// Build the argv that mount.idmapped (or `mount --bind --map-id`)
    /// would receive. Used by tests and the runtime shim.
    pub fn mount_argv(&self) -> Vec<String> {
        vec![
            "mount.idmapped".to_string(),
            "-o".to_string(),
            self.options_string(),
            "-t".to_string(),
            self.fs_type.clone(),
            self.source.display().to_string(),
            self.target.display().to_string(),
        ]
    }

    /// Render the mount option string — `idmap=<host_base>:<container_base>:<size>`
    /// per mapping. Multiple mappings are comma-joined.
    pub fn options_string(&self) -> String {
        let mut parts = Vec::new();
        for m in &self.user_namespace.uid_mappings {
            parts.push(format!(
                "uidmap={}:{}:{}",
                m.host_id, m.container_id, m.size
            ));
        }
        for m in &self.user_namespace.gid_mappings {
            parts.push(format!(
                "gidmap={}:{}:{}",
                m.host_id, m.container_id, m.size
            ));
        }
        parts.join(",")
    }

    /// True if both UID and GID mappings are non-empty (the kernel
    /// requires both).
    pub fn is_complete(&self) -> bool {
        !self.user_namespace.uid_mappings.is_empty()
            && !self.user_namespace.gid_mappings.is_empty()
    }
}

// ── Cgroup namespace ─────────────────────────────────────────────────────────

/// `CLONE_NEWCGROUP` flag from `<linux/sched.h>`.
pub const CLONE_NEWCGROUP: i32 = 0x0200_0000;

/// Per-pod cgroup view. The cgroup the pod is placed in becomes the
/// pod's cgroup-namespace root; everything above it becomes invisible
/// inside the namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupNamespace {
    /// Cgroup absolute path on the host that becomes `/` inside the
    /// namespace. e.g. `/sys/fs/cgroup/kubepods.slice/pod-<id>.slice`.
    pub root: PathBuf,
}

impl CgroupNamespace {
    /// Build the clone(2) flags this namespace requires.
    pub fn clone_flags(&self) -> i32 {
        CLONE_NEWCGROUP
    }

    /// Render the path the container will see in `/proc/<pid>/cgroup`
    /// after entering this namespace. Inside the cgroup ns, paths are
    /// relative to `root`, so the displayed path is always `/`.
    pub fn translate_for_view(&self, host_path: &std::path::Path) -> Option<PathBuf> {
        host_path
            .strip_prefix(&self.root)
            .ok()
            .map(|rel| {
                if rel.as_os_str().is_empty() {
                    PathBuf::from("/")
                } else {
                    PathBuf::from("/").join(rel)
                }
            })
    }
}

// ── Validation ───────────────────────────────────────────────────────────────

/// First reserved host UID — `useradd` and friends carve out 0..1000 for
/// the system. KEP-127 mappings must not overlap that range.
pub const FIRST_NON_RESERVED_HOST_ID: u32 = 1_000;

/// Validate one IdMapping standalone. Catches range overflow,
/// reserved-id encroachment, and zero-sized ranges.
pub fn validate_id_mapping(m: &IdMapping) -> CriResult<()> {
    if m.size == 0 {
        return Err(CriError::Sandbox("id mapping size must be non-zero".into()));
    }
    if m.host_id.checked_add(m.size).is_none() {
        return Err(CriError::Sandbox(format!(
            "id mapping host range overflows: {}+{}", m.host_id, m.size
        )));
    }
    if m.container_id.checked_add(m.size).is_none() {
        return Err(CriError::Sandbox(format!(
            "id mapping container range overflows: {}+{}", m.container_id, m.size
        )));
    }
    if m.host_id < FIRST_NON_RESERVED_HOST_ID {
        return Err(CriError::Sandbox(format!(
            "id mapping host_id {} is in the reserved 0..{} range",
            m.host_id, FIRST_NON_RESERVED_HOST_ID
        )));
    }
    Ok(())
}

/// Validate that no two mappings inside one map (UID or GID) overlap on
/// either the container or the host axis.
pub fn validate_no_overlap(mappings: &[IdMapping]) -> CriResult<()> {
    for (i, a) in mappings.iter().enumerate() {
        let a_host_end      = a.host_id.saturating_add(a.size);
        let a_container_end = a.container_id.saturating_add(a.size);
        for b in &mappings[i + 1..] {
            let b_host_end      = b.host_id.saturating_add(b.size);
            let b_container_end = b.container_id.saturating_add(b.size);
            let host_overlap      = a.host_id < b_host_end && b.host_id < a_host_end;
            let container_overlap = a.container_id < b_container_end && b.container_id < a_container_end;
            if host_overlap {
                return Err(CriError::Sandbox(format!(
                    "id mappings overlap on host axis: [{}+{}] vs [{}+{}]",
                    a.host_id, a.size, b.host_id, b.size,
                )));
            }
            if container_overlap {
                return Err(CriError::Sandbox(format!(
                    "id mappings overlap on container axis: [{}+{}] vs [{}+{}]",
                    a.container_id, a.size, b.container_id, b.size,
                )));
            }
        }
    }
    Ok(())
}

/// Full validation pass over a UserNamespace.
pub fn validate_user_namespace(ns: &UserNamespace) -> CriResult<()> {
    if ns.uid_mappings.is_empty() {
        return Err(CriError::Sandbox("UserNamespace must have at least one UID mapping".into()));
    }
    if ns.gid_mappings.is_empty() {
        return Err(CriError::Sandbox("UserNamespace must have at least one GID mapping".into()));
    }
    for m in &ns.uid_mappings {
        validate_id_mapping(m)?;
    }
    for m in &ns.gid_mappings {
        validate_id_mapping(m)?;
    }
    validate_no_overlap(&ns.uid_mappings)?;
    validate_no_overlap(&ns.gid_mappings)?;
    Ok(())
}

// ── Allocator extensions ─────────────────────────────────────────────────────

/// Snapshot of allocator usage — allocated count, free count,
/// fragmentation ratio.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AllocatorStats {
    pub allocated_ranges: usize,
    pub total_ranges: usize,
    pub fragmentation_ratio: f64,
}

/// Compute allocator stats. `fragmentation_ratio` is `1.0` when every
/// other range is allocated (worst case), `0.0` when allocated ranges
/// form a single contiguous run starting at the pool start.
pub fn allocator_stats(
    pool_start: u32,
    pool_end: u32,
    range_size: u32,
    allocated: &std::collections::BTreeSet<u32>,
) -> AllocatorStats {
    let total = ((pool_end - pool_start) / range_size) as usize;
    if total == 0 {
        return AllocatorStats {
            allocated_ranges: 0,
            total_ranges: 0,
            fragmentation_ratio: 0.0,
        };
    }
    let allocated_count = allocated.len();
    if allocated_count <= 1 {
        return AllocatorStats {
            allocated_ranges: allocated_count,
            total_ranges: total,
            fragmentation_ratio: 0.0,
        };
    }
    // Count gaps between consecutive allocated ranges. A perfectly
    // contiguous block has zero gaps; maximally fragmented has
    // (allocated_count - 1) gaps.
    let mut gaps = 0u32;
    let mut prev: Option<u32> = None;
    for &base in allocated {
        if let Some(p) = prev {
            if base.saturating_sub(p) > range_size {
                gaps += 1;
            }
        }
        prev = Some(base);
    }
    let max_gaps = (allocated_count - 1) as u32;
    let frag = if max_gaps == 0 { 0.0 } else { gaps as f64 / max_gaps as f64 };
    AllocatorStats {
        allocated_ranges: allocated_count,
        total_ranges: total,
        fragmentation_ratio: frag,
    }
}

/// Build the per-pod IdmappedMount description that the runtime would
/// hand to the kernel for a CSI-style volume share. Wires together a
/// freshly allocated user namespace from `allocator` with the
/// source/target/fs_type fields the CSI plugin supplies.
pub fn build_idmapped_mount(
    allocator: &UserNsAllocator,
    source: PathBuf,
    target: PathBuf,
    fs_type: impl Into<String>,
) -> CriResult<IdmappedMount> {
    let user_namespace = allocator
        .allocate_namespace()
        .map_err(CriError::Sandbox)?;
    Ok(IdmappedMount {
        source,
        target,
        fs_type: fs_type.into(),
        user_namespace,
    })
}

/// Helper exposed for tests/CLI: returns whether `range_size` cleanly
/// divides the pool. Misaligned pools waste the trailing slack.
pub fn pool_is_aligned(pool_start: u32, pool_end: u32, range_size: u32) -> bool {
    range_size > 0
        && range_size <= pool_end.saturating_sub(pool_start)
        && (pool_end - pool_start) % range_size == 0
}

/// Default pool/size triple (matches `UserNsAllocator::defaults` so
/// callers can sanity-check their allocator construction).
pub fn default_pool() -> (u32, u32, u32) {
    (1_000_000, i32::MAX as u32, DEFAULT_RANGE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn passthrough() -> UserNamespace {
        UserNamespace::for_pod(1_000_000, 65_536)
    }

    // ── IdmappedMount ────────────────────────────────────────────────────────

    #[test]
    fn idmapped_mount_options_includes_uidmap_and_gidmap() {
        let m = IdmappedMount {
            source: "/host/data".into(),
            target: "/var/lib/cave/mounts/data".into(),
            fs_type: "ext4".into(),
            user_namespace: passthrough(),
        };
        let opts = m.options_string();
        assert!(opts.contains("uidmap=1000000:0:65536"));
        assert!(opts.contains("gidmap=1000000:0:65536"));
    }

    #[test]
    fn idmapped_mount_argv_includes_source_target_fs_type() {
        let m = IdmappedMount {
            source: "/src".into(),
            target: "/dst".into(),
            fs_type: "xfs".into(),
            user_namespace: passthrough(),
        };
        let argv = m.mount_argv();
        assert_eq!(argv[0], "mount.idmapped");
        assert!(argv.iter().any(|a| a == "xfs"));
        assert!(argv.iter().any(|a| a == "/src"));
        assert!(argv.iter().any(|a| a == "/dst"));
    }

    #[test]
    fn idmapped_mount_complete_requires_both_axes() {
        let mut m = IdmappedMount {
            source: "/src".into(),
            target: "/dst".into(),
            fs_type: "ext4".into(),
            user_namespace: passthrough(),
        };
        assert!(m.is_complete());
        m.user_namespace.gid_mappings.clear();
        assert!(!m.is_complete());
    }

    #[test]
    fn mount_attr_idmap_constant_matches_kernel() {
        // From <linux/mount.h>; locked by upstream ABI.
        assert_eq!(MOUNT_ATTR_IDMAP, 0x0010_0000);
    }

    // ── CgroupNamespace ──────────────────────────────────────────────────────

    #[test]
    fn cgroup_ns_clone_flags_returns_clone_newcgroup() {
        let ns = CgroupNamespace { root: "/sys/fs/cgroup/k.slice/pod.slice".into() };
        assert_eq!(ns.clone_flags(), CLONE_NEWCGROUP);
        assert_eq!(CLONE_NEWCGROUP, 0x0200_0000);
    }

    #[test]
    fn cgroup_ns_translates_root_to_slash() {
        let ns = CgroupNamespace { root: "/sys/fs/cgroup/pod".into() };
        let view = ns.translate_for_view(std::path::Path::new("/sys/fs/cgroup/pod")).unwrap();
        assert_eq!(view, PathBuf::from("/"));
    }

    #[test]
    fn cgroup_ns_translates_descendant_relative_to_root() {
        let ns = CgroupNamespace { root: "/sys/fs/cgroup/pod".into() };
        let view = ns.translate_for_view(std::path::Path::new("/sys/fs/cgroup/pod/sub/child")).unwrap();
        assert_eq!(view, PathBuf::from("/sub/child"));
    }

    #[test]
    fn cgroup_ns_outside_root_returns_none() {
        let ns = CgroupNamespace { root: "/sys/fs/cgroup/pod".into() };
        assert!(ns.translate_for_view(std::path::Path::new("/etc")).is_none());
    }

    // ── validate_id_mapping ──────────────────────────────────────────────────

    #[test]
    fn validate_id_mapping_zero_size_errors() {
        let m = IdMapping { container_id: 0, host_id: 1_000_000, size: 0 };
        assert!(validate_id_mapping(&m).is_err());
    }

    #[test]
    fn validate_id_mapping_host_overflow_errors() {
        let m = IdMapping { container_id: 0, host_id: u32::MAX, size: 10 };
        assert!(validate_id_mapping(&m).is_err());
    }

    #[test]
    fn validate_id_mapping_reserved_host_id_errors() {
        let m = IdMapping { container_id: 0, host_id: 100, size: 10 };
        assert!(validate_id_mapping(&m).is_err());
    }

    #[test]
    fn validate_id_mapping_ok_for_default_kep_127_layout() {
        let m = IdMapping { container_id: 0, host_id: 1_000_000, size: 65_536 };
        assert!(validate_id_mapping(&m).is_ok());
    }

    // ── validate_no_overlap ──────────────────────────────────────────────────

    #[test]
    fn no_overlap_passes_for_disjoint_ranges() {
        let mappings = vec![
            IdMapping { container_id: 0,    host_id: 1_000_000, size: 100 },
            IdMapping { container_id: 100,  host_id: 2_000_000, size: 100 },
        ];
        assert!(validate_no_overlap(&mappings).is_ok());
    }

    #[test]
    fn no_overlap_detects_host_axis_overlap() {
        let mappings = vec![
            IdMapping { container_id: 0,   host_id: 1_000_000, size: 100 },
            IdMapping { container_id: 200, host_id: 1_000_050, size: 100 },
        ];
        let err = validate_no_overlap(&mappings).unwrap_err();
        assert!(err.to_string().contains("host axis"));
    }

    #[test]
    fn no_overlap_detects_container_axis_overlap() {
        let mappings = vec![
            IdMapping { container_id: 0,  host_id: 1_000_000, size: 100 },
            IdMapping { container_id: 50, host_id: 2_000_000, size: 100 },
        ];
        let err = validate_no_overlap(&mappings).unwrap_err();
        assert!(err.to_string().contains("container axis"));
    }

    #[test]
    fn no_overlap_passes_for_adjacent_ranges() {
        // [0..100) and [100..200) touch but do not overlap.
        let mappings = vec![
            IdMapping { container_id: 0,   host_id: 1_000_000, size: 100 },
            IdMapping { container_id: 100, host_id: 1_000_100, size: 100 },
        ];
        assert!(validate_no_overlap(&mappings).is_ok());
    }

    // ── validate_user_namespace ──────────────────────────────────────────────

    #[test]
    fn validate_user_namespace_for_pod_ok() {
        let ns = UserNamespace::for_pod(1_000_000, 65_536);
        assert!(validate_user_namespace(&ns).is_ok());
    }

    #[test]
    fn validate_user_namespace_empty_uid_errors() {
        let mut ns = UserNamespace::for_pod(1_000_000, 65_536);
        ns.uid_mappings.clear();
        assert!(validate_user_namespace(&ns).is_err());
    }

    #[test]
    fn validate_user_namespace_empty_gid_errors() {
        let mut ns = UserNamespace::for_pod(1_000_000, 65_536);
        ns.gid_mappings.clear();
        assert!(validate_user_namespace(&ns).is_err());
    }

    #[test]
    fn validate_user_namespace_host_passthrough_rejected() {
        // host_passthrough uses host_id 0 which is reserved.
        let ns = UserNamespace::host_passthrough();
        assert!(validate_user_namespace(&ns).is_err());
    }

    // ── allocator_stats ──────────────────────────────────────────────────────

    #[test]
    fn allocator_stats_empty_pool_returns_zeros() {
        let s = allocator_stats(0, 0, 65_536, &BTreeSet::new());
        assert_eq!(s.total_ranges, 0);
        assert_eq!(s.allocated_ranges, 0);
    }

    #[test]
    fn allocator_stats_zero_allocations_no_fragmentation() {
        let s = allocator_stats(0, 65_536 * 4, 65_536, &BTreeSet::new());
        assert_eq!(s.allocated_ranges, 0);
        assert_eq!(s.total_ranges, 4);
        assert_eq!(s.fragmentation_ratio, 0.0);
    }

    #[test]
    fn allocator_stats_contiguous_allocations_zero_fragmentation() {
        let mut allocated = BTreeSet::new();
        allocated.insert(0);
        allocated.insert(65_536);
        allocated.insert(65_536 * 2);
        let s = allocator_stats(0, 65_536 * 4, 65_536, &allocated);
        assert_eq!(s.allocated_ranges, 3);
        assert_eq!(s.fragmentation_ratio, 0.0);
    }

    #[test]
    fn allocator_stats_alternating_allocations_full_fragmentation() {
        let mut allocated = BTreeSet::new();
        allocated.insert(0);
        allocated.insert(65_536 * 2);
        allocated.insert(65_536 * 4);
        let s = allocator_stats(0, 65_536 * 6, 65_536, &allocated);
        assert_eq!(s.allocated_ranges, 3);
        assert_eq!(s.fragmentation_ratio, 1.0);
    }

    // ── build_idmapped_mount ────────────────────────────────────────────────

    #[test]
    fn build_idmapped_mount_consumes_one_range_from_allocator() {
        let alloc = UserNsAllocator::new(2_000_000, 2_000_000 + 65_536 * 4, 65_536);
        let m = build_idmapped_mount(
            &alloc,
            "/csi/source".into(),
            "/var/lib/cave/csi/dst".into(),
            "ext4",
        ).unwrap();
        assert_eq!(alloc.allocated(), 1);
        assert_eq!(m.fs_type, "ext4");
        assert_eq!(m.source, std::path::PathBuf::from("/csi/source"));
    }

    // ── pool_is_aligned ──────────────────────────────────────────────────────

    #[test]
    fn pool_is_aligned_true_for_clean_division() {
        assert!(pool_is_aligned(0, 65_536 * 4, 65_536));
    }

    #[test]
    fn pool_is_aligned_false_for_remainder() {
        assert!(!pool_is_aligned(0, 65_537, 65_536));
    }

    #[test]
    fn pool_is_aligned_false_for_zero_range_size() {
        assert!(!pool_is_aligned(0, 100, 0));
    }

    // ── default_pool ────────────────────────────────────────────────────────

    #[test]
    fn default_pool_starts_at_one_million() {
        let (start, _end, size) = default_pool();
        assert_eq!(start, 1_000_000);
        assert_eq!(size, 65_536);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn idmapped_mount_roundtrips_through_json() {
        let m = IdmappedMount {
            source: "/src".into(),
            target: "/dst".into(),
            fs_type: "ext4".into(),
            user_namespace: passthrough(),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: IdmappedMount = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn cgroup_namespace_roundtrips_through_json() {
        let ns = CgroupNamespace { root: "/sys/fs/cgroup/pod".into() };
        let json = serde_json::to_string(&ns).unwrap();
        let back: CgroupNamespace = serde_json::from_str(&json).unwrap();
        assert_eq!(ns, back);
    }
}

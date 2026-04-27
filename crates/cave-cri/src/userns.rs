//! User namespace support — KEP-127 / Pod-level user namespaces.
//!
//! When `pod.spec.hostUsers: false`, the kubelet asks the runtime to put
//! the pod in a private user namespace, mapping container UID/GID 0 to
//! a high-numbered, per-pod range on the host. cave-cri implements the
//! containerd-side wiring:
//!
//! - Allocate a 65536-wide host range from `/etc/subuid` / `/etc/subgid`
//!   (or a configured pool) per pod.
//! - Render the `/proc/self/uid_map` and `/proc/self/gid_map` payloads.
//! - Encode the OCI runtime-spec `linux.uidMappings` /
//!   `linux.gidMappings` arrays.
//! - Translate "container UID 1000" → "host UID base+1000" both ways.
//!
//! Upstream:
//! - kubernetes KEP-127:
//!   <https://github.com/kubernetes/enhancements/tree/master/keps/sig-node/127-user-namespaces>
//! - containerd: `pkg/cri/server/podsandbox/userns_linux.go`
//! - runc:       `libcontainer/userns/userns.go`

use serde::{Deserialize, Serialize};

/// Default range size containerd allocates per pod (`USERNS_RANGE_SIZE`).
pub const DEFAULT_RANGE_SIZE: u32 = 65_536;

/// Default first host UID/GID to hand out from. Matches the lower bound
/// containerd uses when the operator hasn't carved out a custom pool —
/// well above the 0..1000 host accounts and the typical 100k user-account
/// range from `useradd`.
pub const DEFAULT_FIRST_HOST_ID: u32 = 1_000_000;

/// One contiguous mapping line — same shape as a row in
/// `/proc/<pid>/uid_map` and `linux.uidMappings[]` in the OCI spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdMapping {
    /// First UID/GID inside the namespace.
    pub container_id: u32,
    /// First UID/GID outside the namespace (i.e. on the host).
    pub host_id: u32,
    /// Number of consecutive IDs covered by this mapping.
    pub size: u32,
}

impl IdMapping {
    /// Render this mapping in `/proc/<pid>/uid_map` wire format —
    /// space-separated `container host size` triple.
    pub fn render_proc_line(&self) -> String {
        format!("{} {} {}", self.container_id, self.host_id, self.size)
    }

    /// Translate a container-side UID/GID to its host counterpart, or
    /// `None` if it falls outside this mapping's range.
    pub fn translate_to_host(&self, container: u32) -> Option<u32> {
        if container < self.container_id {
            return None;
        }
        let offset = container - self.container_id;
        if offset >= self.size {
            return None;
        }
        Some(self.host_id + offset)
    }

    /// Translate a host UID/GID back into the container's namespace.
    pub fn translate_to_container(&self, host: u32) -> Option<u32> {
        if host < self.host_id {
            return None;
        }
        let offset = host - self.host_id;
        if offset >= self.size {
            return None;
        }
        Some(self.container_id + offset)
    }

    /// True if the mapping covers `container`.
    pub fn covers_container(&self, container: u32) -> bool {
        self.translate_to_host(container).is_some()
    }
}

/// Per-pod user namespace configuration. Holds both the UID and GID map
/// (containerd allocates the same range for both by default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserNamespace {
    pub uid_mappings: Vec<IdMapping>,
    pub gid_mappings: Vec<IdMapping>,
}

impl UserNamespace {
    /// Build a "host" namespace — no remapping, identity 1:1 over the
    /// full ID space. Used when `hostUsers: true`.
    pub fn host_passthrough() -> Self {
        let identity = IdMapping { container_id: 0, host_id: 0, size: u32::MAX };
        Self {
            uid_mappings: vec![identity],
            gid_mappings: vec![identity],
        }
    }

    /// Build the standard KEP-127 mapping: container `[0, range_size)`
    /// → host `[base, base+range_size)` for both UID and GID.
    pub fn for_pod(host_base: u32, range_size: u32) -> Self {
        let m = IdMapping { container_id: 0, host_id: host_base, size: range_size };
        Self {
            uid_mappings: vec![m],
            gid_mappings: vec![m],
        }
    }

    /// True if every mapping is `0 → 0 → u32::MAX` (i.e. host-passthrough).
    pub fn is_host(&self) -> bool {
        let identity = IdMapping { container_id: 0, host_id: 0, size: u32::MAX };
        self.uid_mappings == vec![identity] && self.gid_mappings == vec![identity]
    }

    /// Render the `/proc/<pid>/uid_map` payload — one mapping per line.
    pub fn render_uid_map_file(&self) -> String {
        Self::render_map(&self.uid_mappings)
    }

    /// Render the `/proc/<pid>/gid_map` payload.
    pub fn render_gid_map_file(&self) -> String {
        Self::render_map(&self.gid_mappings)
    }

    fn render_map(mappings: &[IdMapping]) -> String {
        mappings
            .iter()
            .map(|m| m.render_proc_line())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
}

/// Allocator for non-overlapping per-pod ranges drawn from a configured
/// host pool. Mirrors containerd's `userns.NewSubidStore`.
#[derive(Debug)]
pub struct UserNsAllocator {
    pool_start: u32,
    pool_end: u32,
    range_size: u32,
    used: std::sync::Mutex<std::collections::BTreeSet<u32>>,
}

impl UserNsAllocator {
    /// Build an allocator that hands out `range_size`-wide blocks from
    /// `[pool_start, pool_end)`.
    pub fn new(pool_start: u32, pool_end: u32, range_size: u32) -> Self {
        Self {
            pool_start,
            pool_end,
            range_size,
            used: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    /// Build the default allocator — 1M..2^31, 65536 wide ranges.
    pub fn defaults() -> Self {
        Self::new(DEFAULT_FIRST_HOST_ID, i32::MAX as u32, DEFAULT_RANGE_SIZE)
    }

    /// Reserve the next free range. Returns the host base UID/GID.
    pub fn allocate(&self) -> Result<u32, String> {
        let mut used = self.used.lock().unwrap();
        let mut candidate = self.pool_start;
        while candidate.saturating_add(self.range_size) <= self.pool_end {
            if !used.contains(&candidate) {
                used.insert(candidate);
                return Ok(candidate);
            }
            candidate = candidate.saturating_add(self.range_size);
        }
        Err("user namespace pool exhausted".into())
    }

    /// Release a previously-reserved range.
    pub fn release(&self, host_base: u32) {
        self.used.lock().unwrap().remove(&host_base);
    }

    /// Number of ranges currently checked out.
    pub fn allocated(&self) -> usize {
        self.used.lock().unwrap().len()
    }

    /// Build a UserNamespace for one pod, allocating a fresh range.
    pub fn allocate_namespace(&self) -> Result<UserNamespace, String> {
        let base = self.allocate()?;
        Ok(UserNamespace::for_pod(base, self.range_size))
    }
}

/// Parse the `/etc/subuid` (or `/etc/subgid`) file format —
/// `<user>:<host_start>:<count>` lines — into the matching mappings for
/// `username`.
pub fn parse_subid_file(content: &str, username: &str) -> Vec<IdMapping> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() != 3 {
            continue;
        }
        if parts[0] != username {
            continue;
        }
        let host_id = match parts[1].parse::<u32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let size = match parts[2].parse::<u32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        out.push(IdMapping { container_id: 0, host_id, size });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── IdMapping ────────────────────────────────────────────────────────────

    #[test]
    fn render_proc_line_uses_three_space_separated_ints() {
        let m = IdMapping { container_id: 0, host_id: 100_000, size: 65_536 };
        assert_eq!(m.render_proc_line(), "0 100000 65536");
    }

    #[test]
    fn translate_to_host_inside_range_offsets_base() {
        let m = IdMapping { container_id: 0, host_id: 1_000_000, size: 100 };
        assert_eq!(m.translate_to_host(0),  Some(1_000_000));
        assert_eq!(m.translate_to_host(50), Some(1_000_050));
        assert_eq!(m.translate_to_host(99), Some(1_000_099));
    }

    #[test]
    fn translate_to_host_outside_range_is_none() {
        let m = IdMapping { container_id: 0, host_id: 1_000_000, size: 100 };
        assert!(m.translate_to_host(100).is_none());
        assert!(m.translate_to_host(500).is_none());
    }

    #[test]
    fn translate_to_host_below_container_id_is_none() {
        let m = IdMapping { container_id: 1000, host_id: 1_000_000, size: 100 };
        assert!(m.translate_to_host(500).is_none());
        assert!(m.translate_to_host(999).is_none());
        assert_eq!(m.translate_to_host(1000), Some(1_000_000));
    }

    #[test]
    fn translate_to_container_inverts_translate_to_host() {
        let m = IdMapping { container_id: 0, host_id: 1_000_000, size: 100 };
        assert_eq!(m.translate_to_container(1_000_050), Some(50));
        assert!(m.translate_to_container(999_999).is_none());
        assert!(m.translate_to_container(1_000_100).is_none());
    }

    #[test]
    fn covers_container_matches_translate() {
        let m = IdMapping { container_id: 0, host_id: 100, size: 10 };
        for c in 0..10 { assert!(m.covers_container(c)); }
        assert!(!m.covers_container(10));
    }

    // ── UserNamespace ────────────────────────────────────────────────────────

    #[test]
    fn host_passthrough_is_identity() {
        let ns = UserNamespace::host_passthrough();
        assert!(ns.is_host());
        assert_eq!(ns.uid_mappings[0].translate_to_host(1234), Some(1234));
    }

    #[test]
    fn for_pod_creates_kep_127_mapping() {
        let ns = UserNamespace::for_pod(1_000_000, 65_536);
        assert!(!ns.is_host());
        assert_eq!(ns.uid_mappings.len(), 1);
        assert_eq!(ns.uid_mappings[0].translate_to_host(0), Some(1_000_000));
        assert_eq!(ns.uid_mappings[0].translate_to_host(65_535), Some(1_065_535));
        assert!(ns.uid_mappings[0].translate_to_host(65_536).is_none());
    }

    #[test]
    fn for_pod_uses_same_range_for_uid_and_gid() {
        let ns = UserNamespace::for_pod(2_000_000, 100);
        assert_eq!(ns.uid_mappings, ns.gid_mappings);
    }

    #[test]
    fn render_uid_map_file_appends_newline() {
        let ns = UserNamespace::for_pod(100, 50);
        let s = ns.render_uid_map_file();
        assert!(s.ends_with('\n'));
        assert!(s.contains("0 100 50"));
    }

    #[test]
    fn render_uid_map_file_handles_multiple_mappings() {
        let ns = UserNamespace {
            uid_mappings: vec![
                IdMapping { container_id: 0, host_id: 100_000, size: 1 },
                IdMapping { container_id: 1, host_id: 200_000, size: 65_535 },
            ],
            gid_mappings: vec![],
        };
        let s = ns.render_uid_map_file();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines, vec!["0 100000 1", "1 200000 65535"]);
    }

    // ── UserNsAllocator ──────────────────────────────────────────────────────

    #[test]
    fn allocator_hands_out_non_overlapping_ranges() {
        let a = UserNsAllocator::new(1_000_000, 1_000_000 + 65_536 * 4, 65_536);
        let r1 = a.allocate().unwrap();
        let r2 = a.allocate().unwrap();
        let r3 = a.allocate().unwrap();
        assert_eq!(r1, 1_000_000);
        assert_eq!(r2, 1_065_536);
        assert_eq!(r3, 1_131_072);
    }

    #[test]
    fn allocator_recycles_released_range() {
        let a = UserNsAllocator::new(1_000_000, 1_000_000 + 65_536 * 2, 65_536);
        let r1 = a.allocate().unwrap();
        let _r2 = a.allocate().unwrap();
        a.release(r1);
        let r3 = a.allocate().unwrap();
        assert_eq!(r3, r1);
    }

    #[test]
    fn allocator_exhausted_returns_error() {
        let a = UserNsAllocator::new(0, 100, 50);
        assert!(a.allocate().is_ok());
        assert!(a.allocate().is_ok());
        let err = a.allocate().unwrap_err();
        assert!(err.contains("exhausted"));
    }

    #[test]
    fn allocator_default_pool_starts_at_one_million() {
        let a = UserNsAllocator::defaults();
        assert_eq!(a.allocate().unwrap(), DEFAULT_FIRST_HOST_ID);
    }

    #[test]
    fn allocator_allocated_count_tracks_outstanding() {
        let a = UserNsAllocator::new(0, 1000, 100);
        assert_eq!(a.allocated(), 0);
        a.allocate().unwrap();
        a.allocate().unwrap();
        assert_eq!(a.allocated(), 2);
    }

    #[test]
    fn allocate_namespace_returns_for_pod_mapping() {
        let a = UserNsAllocator::new(500_000, 500_000 + 65_536, 65_536);
        let ns = a.allocate_namespace().unwrap();
        assert_eq!(ns.uid_mappings[0].host_id, 500_000);
        assert_eq!(ns.uid_mappings[0].size, 65_536);
    }

    #[test]
    fn allocator_concurrent_allocate_is_safe() {
        use std::sync::Arc;
        let a = Arc::new(UserNsAllocator::new(0, 65_536 * 16, 65_536));
        let mut handles = vec![];
        for _ in 0..16 {
            let a = a.clone();
            handles.push(std::thread::spawn(move || a.allocate().unwrap()));
        }
        let mut bases: Vec<u32> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        bases.sort();
        bases.dedup();
        assert_eq!(bases.len(), 16, "every concurrent allocation must be unique");
    }

    // ── parse_subid_file ─────────────────────────────────────────────────────

    #[test]
    fn parse_subid_returns_matching_user_lines() {
        let content = "alice:100000:65536\nbob:200000:65536\nalice:1000000:1000\n";
        let alice = parse_subid_file(content, "alice");
        assert_eq!(alice.len(), 2);
        assert_eq!(alice[0].host_id, 100_000);
        assert_eq!(alice[0].size, 65_536);
        assert_eq!(alice[1].host_id, 1_000_000);
        let bob = parse_subid_file(content, "bob");
        assert_eq!(bob.len(), 1);
    }

    #[test]
    fn parse_subid_skips_comments_and_blank_lines() {
        let content = "# comment line\n\n   \nalice:100000:65536\n# another\n";
        let mappings = parse_subid_file(content, "alice");
        assert_eq!(mappings.len(), 1);
    }

    #[test]
    fn parse_subid_skips_malformed_lines() {
        let content = "garbage\nalice:notanumber:65536\nalice:100000:65536\n";
        let mappings = parse_subid_file(content, "alice");
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].host_id, 100_000);
    }

    #[test]
    fn parse_subid_unknown_user_returns_empty() {
        let content = "alice:100000:65536\n";
        assert!(parse_subid_file(content, "ghost").is_empty());
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn user_namespace_roundtrips_through_json() {
        let ns = UserNamespace::for_pod(1_000_000, 65_536);
        let json = serde_json::to_string(&ns).unwrap();
        let back: UserNamespace = serde_json::from_str(&json).unwrap();
        assert_eq!(ns, back);
    }

    #[test]
    fn id_mapping_serializes_with_lowercase_field_names() {
        let m = IdMapping { container_id: 1, host_id: 2, size: 3 };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"container_id\":1"));
        assert!(json.contains("\"host_id\":2"));
        assert!(json.contains("\"size\":3"));
    }
}

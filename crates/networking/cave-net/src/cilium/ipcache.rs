// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! IPCache — IP → (identity, encryption-key, tunnel-endpoint) map.
//!
//! Mirrors `pkg/ipcache/ipcache.go` plus the BPF map shape from
//! `bpf/lib/ipcache.h`. This is the cluster-wide table the datapath
//! consults to resolve a remote IP to:
//!
//! * the **numeric identity** (used by the policy engine),
//! * an optional **encryption-key index** (for IPsec),
//! * an optional **tunnel-endpoint** node IP (for VXLAN/Geneve mode).
//!
//! Sources of truth (mirrors upstream `ResourceID` / `Source`):
//!
//! * [`IpcacheSource::Local`] — locally-managed pod (highest priority).
//! * [`IpcacheSource::Generated`] — agent-generated entry (e.g. CIDR).
//! * [`IpcacheSource::ClusterMesh`] — pushed by a peer cluster.
//! * [`IpcacheSource::Kvstore`] — other nodes via the KVStore
//!   (etcd) propagation channel.
//! * [`IpcacheSource::Restored`] — loaded from disk after restart.
//!
//! Conflicts are resolved by source priority (`Local` > `Generated` >
//! `ClusterMesh` > `Kvstore` > `Restored`); a higher-priority source
//! always wins.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IpcacheSource {
    /// Lowest priority.
    Restored = 0,
    Kvstore = 1,
    ClusterMesh = 2,
    Generated = 3,
    /// Highest priority.
    Local = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcacheEntry {
    pub identity: u32,
    pub source: IpcacheSource,
    /// Encryption key index (0 = no encryption). Mirrors
    /// `bpf/lib/ipcache.h::remote_endpoint_info.key`.
    pub encryption_key: u8,
    /// Optional tunnel endpoint (the remote node IP) — populated for
    /// pods on remote nodes when running in tunnel mode.
    pub tunnel_endpoint: Option<IpAddr>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IpcacheError {
    #[error("entry for {ip} not found")]
    NotFound { ip: IpAddr },
    #[error("source {existing:?} has higher priority than {incoming:?}")]
    LowerPriority {
        existing: IpcacheSource,
        incoming: IpcacheSource,
    },
    #[error("tenant {tenant} cannot mutate ipcache owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct Ipcache {
    pub tenant: TenantId,
    entries: HashMap<IpAddr, IpcacheEntry>,
}

impl Ipcache {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            entries: HashMap::new(),
        }
    }

    /// Upsert an entry. If a higher-priority source already owns the IP,
    /// returns `LowerPriority`. Otherwise the entry is written.
    pub fn upsert(&mut self, ip: IpAddr, entry: IpcacheEntry) -> Result<(), IpcacheError> {
        if let Some(existing) = self.entries.get(&ip) {
            if existing.source > entry.source {
                return Err(IpcacheError::LowerPriority {
                    existing: existing.source,
                    incoming: entry.source,
                });
            }
        }
        self.entries.insert(ip, entry);
        Ok(())
    }

    /// Force-write an entry regardless of priority. Used when a pod
    /// is deleted and the entry must be replaced by a remote source.
    pub fn force_set(&mut self, ip: IpAddr, entry: IpcacheEntry) {
        self.entries.insert(ip, entry);
    }

    pub fn lookup(&self, ip: IpAddr) -> Option<&IpcacheEntry> {
        self.entries.get(&ip)
    }

    /// Remove an entry only if its source matches `expected_source`
    /// (so the local agent doesn't accidentally drop a Local entry
    /// when processing a stale Kvstore tombstone).
    pub fn remove_if_source(&mut self, ip: IpAddr, expected_source: IpcacheSource) -> bool {
        match self.entries.get(&ip) {
            Some(e) if e.source == expected_source => {
                self.entries.remove(&ip);
                true
            }
            _ => false,
        }
    }

    pub fn remove_force(&mut self, ip: IpAddr) -> Option<IpcacheEntry> {
        self.entries.remove(&ip)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolve identity for an IP. Mirrors `bpf/lib/ipcache.h::lookup_ip4_remote_endpoint`.
    pub fn identity_of(&self, ip: IpAddr) -> Option<u32> {
        self.entries.get(&ip).map(|e| e.identity)
    }

    /// Iterate by-source for diagnostics (mirrors `cilium ipcache list`).
    pub fn list_by_source(&self, source: IpcacheSource) -> Vec<(IpAddr, IpcacheEntry)> {
        self.entries
            .iter()
            .filter(|(_, v)| v.source == source)
            .map(|(k, v)| (*k, *v))
            .collect()
    }

    /// Bulk delete every entry from a source (used when a remote
    /// cluster or KVStore connection drops). Returns count removed.
    pub fn purge_source(&mut self, source: IpcacheSource) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, v| v.source != source);
        before - self.entries.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ipcache/ipcache.go", "Ipcache");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn entry(identity: u32, source: IpcacheSource) -> IpcacheEntry {
        IpcacheEntry {
            identity,
            source,
            encryption_key: 0,
            tunnel_endpoint: None,
        }
    }

    // ── Source priority ─────────────────────────────────────────────────────

    #[test]
    fn ipcache_source_priority_ordering() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Source.Priority",
            "tenant-ipc-prio"
        );
        assert!(IpcacheSource::Local > IpcacheSource::Generated);
        assert!(IpcacheSource::Generated > IpcacheSource::ClusterMesh);
        assert!(IpcacheSource::ClusterMesh > IpcacheSource::Kvstore);
        assert!(IpcacheSource::Kvstore > IpcacheSource::Restored);
    }

    // ── Upsert / lookup ─────────────────────────────────────────────────────

    #[test]
    fn ipcache_upsert_and_lookup_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipcache/ipcache.go", "Upsert", "tenant-ipc-up");
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        assert_eq!(c.lookup(ip(10, 0, 1, 5)).unwrap().identity, 256);
    }

    #[test]
    fn ipcache_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Lookup.NotFound",
            "tenant-ipc-lknf"
        );
        let c = Ipcache::new(tenant);
        assert!(c.lookup(ip(8, 8, 8, 8)).is_none());
    }

    #[test]
    fn ipcache_identity_of_returns_identity() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/lib/ipcache.h",
            "lookup_ip4_remote_endpoint",
            "tenant-ipc-id"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(257, IpcacheSource::Local))
            .unwrap();
        assert_eq!(c.identity_of(ip(10, 0, 1, 5)), Some(257));
        assert_eq!(c.identity_of(ip(8, 8, 8, 8)), None);
    }

    // ── Source priority on upsert ───────────────────────────────────────────

    #[test]
    fn ipcache_upsert_lower_priority_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Upsert.LowerPriority",
            "tenant-ipc-lp"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        let err = c
            .upsert(ip(10, 0, 1, 5), entry(999, IpcacheSource::Kvstore))
            .unwrap_err();
        assert!(matches!(err, IpcacheError::LowerPriority { .. }));
        // Existing entry preserved.
        assert_eq!(c.identity_of(ip(10, 0, 1, 5)), Some(256));
    }

    #[test]
    fn ipcache_upsert_higher_priority_overwrites() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Upsert.HigherPriority",
            "tenant-ipc-hp"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(999, IpcacheSource::Kvstore))
            .unwrap();
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        assert_eq!(c.identity_of(ip(10, 0, 1, 5)), Some(256));
    }

    #[test]
    fn ipcache_upsert_same_priority_overwrites() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Upsert.SamePriority",
            "tenant-ipc-sp"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(100, IpcacheSource::Kvstore))
            .unwrap();
        c.upsert(ip(10, 0, 1, 5), entry(200, IpcacheSource::Kvstore))
            .unwrap();
        assert_eq!(c.identity_of(ip(10, 0, 1, 5)), Some(200));
    }

    // ── Force write ─────────────────────────────────────────────────────────

    #[test]
    fn ipcache_force_set_bypasses_priority() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipcache/ipcache.go", "ForceSet", "tenant-ipc-fs");
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        c.force_set(ip(10, 0, 1, 5), entry(999, IpcacheSource::Kvstore));
        assert_eq!(c.identity_of(ip(10, 0, 1, 5)), Some(999));
    }

    // ── Remove ──────────────────────────────────────────────────────────────

    #[test]
    fn ipcache_remove_if_source_match_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Remove.SourceMatch",
            "tenant-ipc-rmm"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(100, IpcacheSource::Kvstore))
            .unwrap();
        assert!(c.remove_if_source(ip(10, 0, 1, 5), IpcacheSource::Kvstore));
        assert!(c.lookup(ip(10, 0, 1, 5)).is_none());
    }

    #[test]
    fn ipcache_remove_if_source_mismatch_keeps_entry() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Remove.SourceMismatch",
            "tenant-ipc-rmmm"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        assert!(!c.remove_if_source(ip(10, 0, 1, 5), IpcacheSource::Kvstore));
        // Entry still present.
        assert!(c.lookup(ip(10, 0, 1, 5)).is_some());
    }

    #[test]
    fn ipcache_remove_force_drops_any_source() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ipcache/ipcache.go", "Remove.Force", "tenant-ipc-rmf");
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        let removed = c.remove_force(ip(10, 0, 1, 5)).unwrap();
        assert_eq!(removed.identity, 256);
    }

    #[test]
    fn ipcache_remove_force_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Remove.Force.NotFound",
            "tenant-ipc-rmfnf"
        );
        let mut c = Ipcache::new(tenant);
        assert!(c.remove_force(ip(8, 8, 8, 8)).is_none());
    }

    // ── List by source / purge ──────────────────────────────────────────────

    #[test]
    fn ipcache_list_by_source_filters_correctly() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ipcache/ipcache.go", "ListBySource", "tenant-ipc-ls");
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        c.upsert(ip(10, 0, 1, 6), entry(257, IpcacheSource::Local))
            .unwrap();
        c.upsert(ip(10, 0, 1, 7), entry(258, IpcacheSource::Kvstore))
            .unwrap();
        let local = c.list_by_source(IpcacheSource::Local);
        assert_eq!(local.len(), 2);
    }

    #[test]
    fn ipcache_purge_source_drops_all_matching() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/ipcache/ipcache.go", "PurgeSource", "tenant-ipc-pur");
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Kvstore))
            .unwrap();
        c.upsert(ip(10, 0, 1, 6), entry(257, IpcacheSource::Kvstore))
            .unwrap();
        c.upsert(ip(10, 0, 1, 7), entry(258, IpcacheSource::Local))
            .unwrap();
        let n = c.purge_source(IpcacheSource::Kvstore);
        assert_eq!(n, 2);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn ipcache_purge_source_empty_returns_zero() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "PurgeSource.Empty",
            "tenant-ipc-pure"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Local))
            .unwrap();
        assert_eq!(c.purge_source(IpcacheSource::Kvstore), 0);
        assert_eq!(c.len(), 1);
    }

    // ── Encryption key ──────────────────────────────────────────────────────

    #[test]
    fn ipcache_entry_with_encryption_key() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/lib/ipcache.h",
            "remote_endpoint_info.key",
            "tenant-ipc-key"
        );
        let mut c = Ipcache::new(tenant);
        let mut e = entry(256, IpcacheSource::Local);
        e.encryption_key = 7;
        c.upsert(ip(10, 0, 1, 5), e).unwrap();
        assert_eq!(c.lookup(ip(10, 0, 1, 5)).unwrap().encryption_key, 7);
    }

    // ── Tunnel endpoint ─────────────────────────────────────────────────────

    #[test]
    fn ipcache_entry_with_tunnel_endpoint() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/lib/ipcache.h",
            "remote_endpoint_info.tunnel_endpoint",
            "tenant-ipc-tep"
        );
        let mut c = Ipcache::new(tenant);
        let mut e = entry(256, IpcacheSource::Local);
        e.tunnel_endpoint = Some(ip(10, 0, 0, 1));
        c.upsert(ip(10, 244, 1, 5), e).unwrap();
        assert_eq!(
            c.lookup(ip(10, 244, 1, 5)).unwrap().tunnel_endpoint,
            Some(ip(10, 0, 0, 1))
        );
    }

    // ── IPv6 ─────────────────────────────────────────────────────────────────

    #[test]
    fn ipcache_v6_entry_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipcache/ipcache.go", "Upsert.V6", "tenant-ipc-v6");
        let mut c = Ipcache::new(tenant);
        let v6: IpAddr = "fd00:1::5".parse().unwrap();
        c.upsert(v6, entry(256, IpcacheSource::Local)).unwrap();
        assert_eq!(c.identity_of(v6), Some(256));
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn ipcache_len_tracks_inserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipcache/ipcache.go", "Len", "tenant-ipc-len");
        let mut c = Ipcache::new(tenant);
        for i in 0..10u8 {
            c.upsert(ip(10, 0, 1, i), entry(256 + i as u32, IpcacheSource::Local))
                .unwrap();
        }
        assert_eq!(c.len(), 10);
    }

    #[test]
    fn ipcache_is_empty_initially() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipcache/ipcache.go", "IsEmpty", "tenant-ipc-emp");
        let c = Ipcache::new(tenant);
        assert!(c.is_empty());
    }

    // ── Source ordering: edge cases ─────────────────────────────────────────

    #[test]
    fn ipcache_clustermesh_loses_to_generated() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Source.GenVsMesh",
            "tenant-ipc-gvm"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Generated))
            .unwrap();
        let err = c
            .upsert(ip(10, 0, 1, 5), entry(999, IpcacheSource::ClusterMesh))
            .unwrap_err();
        assert!(matches!(err, IpcacheError::LowerPriority { .. }));
    }

    #[test]
    fn ipcache_kvstore_overrides_restored() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Source.KvOverRestored",
            "tenant-ipc-kor"
        );
        let mut c = Ipcache::new(tenant);
        c.upsert(ip(10, 0, 1, 5), entry(256, IpcacheSource::Restored))
            .unwrap();
        c.upsert(ip(10, 0, 1, 5), entry(999, IpcacheSource::Kvstore))
            .unwrap();
        assert_eq!(c.identity_of(ip(10, 0, 1, 5)), Some(999));
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn ipcache_entry_serde_round_trip() {
        let (_c, _t) =
            cilium_test_ctx!("pkg/ipcache/ipcache.go", "Entry.Serde", "tenant-ipc-eserde");
        let e = IpcacheEntry {
            identity: 256,
            source: IpcacheSource::Local,
            encryption_key: 7,
            tunnel_endpoint: Some(ip(10, 0, 0, 1)),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn ipcache_source_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ipcache/ipcache.go",
            "Source.Serde",
            "tenant-ipc-sserde"
        );
        for s in [
            IpcacheSource::Restored,
            IpcacheSource::Kvstore,
            IpcacheSource::ClusterMesh,
            IpcacheSource::Generated,
            IpcacheSource::Local,
        ] {
            let j = serde_json::to_string(&s).unwrap();
            let back: IpcacheSource = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }
}

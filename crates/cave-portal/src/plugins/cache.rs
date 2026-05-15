// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cache wrap — native Redis/Valkey-compatible admin UI.
//!
//! Replaces RedisInsight / Valkey UI for the cave runtime. Tenants browse
//! their own keyspace (filtered by tenant prefix), inspect TTLs, list
//! pub/sub channels and review the cluster slot map. **No** redirect to a
//! vendor UI exists.
//!
//! Panels (per ADR-147 portal contract):
//!   * `dashboard`   — memory used, hit rate, ops/sec, evictions
//!   * `keys`        — paged browser with TTL display
//!   * `pubsub`      — active channels + per-channel publish rate
//!   * `cluster`     — 16,384 slot ownership map

use super::ViewPersona;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Snapshots ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ServerSnapshot {
    pub used_memory_bytes: u64,
    pub used_memory_rss_bytes: u64,
    pub max_memory_bytes: u64,
    pub keys_total: u64,
    pub hits_per_sec: f64,
    pub lookups_per_sec: f64,
    pub ops_per_sec: f64,
    pub evictions_per_sec: f64,
    pub connected_clients: u64,
}

impl ServerSnapshot {
    pub fn hit_rate(&self) -> f64 {
        if self.lookups_per_sec <= 0.0 {
            return 0.0;
        }
        (self.hits_per_sec / self.lookups_per_sec).clamp(0.0, 1.0)
    }

    pub fn fragmentation_ratio(&self) -> f64 {
        if self.used_memory_bytes == 0 {
            return 0.0;
        }
        self.used_memory_rss_bytes as f64 / self.used_memory_bytes as f64
    }

    pub fn memory_utilisation(&self) -> f64 {
        if self.max_memory_bytes == 0 {
            return 0.0;
        }
        (self.used_memory_bytes as f64 / self.max_memory_bytes as f64).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEntry {
    pub key: String,
    pub tenant: String,
    pub kind: KeyKind,
    pub size_bytes: u64,
    /// `None` means no expiry.
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyKind {
    String,
    List,
    Set,
    Zset,
    Hash,
    Stream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubsubChannel {
    pub name: String,
    pub tenant: String,
    pub subscribers: u32,
    pub messages_per_sec: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotRange {
    pub start: u16,
    pub end: u16,
    pub primary_node: String,
    pub replica_nodes: Vec<String>,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CacheError {
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("invalid slot range {0}..={1} (max 16383)")]
    InvalidSlotRange(u16, u16),
    #[error("slot range {0}..={1} overlaps existing range")]
    OverlappingSlotRange(u16, u16),
    #[error("invalid key: {0}")]
    InvalidKey(String),
}

// ── Plugin state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CachePlugin {
    snapshot: ServerSnapshot,
    keys: Vec<KeyEntry>,
    channels: Vec<PubsubChannel>,
    slots: Vec<SlotRange>,
}

impl CachePlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_snapshot(&mut self, snapshot: ServerSnapshot) {
        self.snapshot = snapshot;
    }

    pub fn snapshot(&self) -> &ServerSnapshot {
        &self.snapshot
    }

    pub fn register_key(&mut self, entry: KeyEntry) -> Result<(), CacheError> {
        validate_key(&entry.key)?;
        self.keys.retain(|k| k.key != entry.key);
        self.keys.push(entry);
        Ok(())
    }

    pub fn register_channel(&mut self, ch: PubsubChannel) {
        self.channels.retain(|c| c.name != ch.name);
        self.channels.push(ch);
    }

    pub fn register_slot_range(&mut self, range: SlotRange) -> Result<(), CacheError> {
        if range.start > range.end {
            return Err(CacheError::InvalidSlotRange(range.start, range.end));
        }
        if range.end > 16_383 {
            return Err(CacheError::InvalidSlotRange(range.start, range.end));
        }
        for existing in &self.slots {
            if range.start <= existing.end && existing.start <= range.end {
                return Err(CacheError::OverlappingSlotRange(range.start, range.end));
            }
        }
        self.slots.push(range);
        Ok(())
    }

    pub fn dashboard(&self) -> DashboardPanel {
        DashboardPanel {
            used_memory_bytes: self.snapshot.used_memory_bytes,
            max_memory_bytes: self.snapshot.max_memory_bytes,
            memory_utilisation: self.snapshot.memory_utilisation(),
            hit_rate: self.snapshot.hit_rate(),
            ops_per_sec: self.snapshot.ops_per_sec,
            evictions_per_sec: self.snapshot.evictions_per_sec,
            fragmentation_ratio: self.snapshot.fragmentation_ratio(),
            keys_total: self.snapshot.keys_total,
            connected_clients: self.snapshot.connected_clients,
        }
    }

    /// Browse keys with optional glob match. Tenants only see their own
    /// keys; admins see everything.
    pub fn list_keys(
        &self,
        glob: Option<&str>,
        persona: ViewPersona,
        tenant: &str,
        limit: usize,
    ) -> Vec<&KeyEntry> {
        self.keys
            .iter()
            .filter(|k| persona == ViewPersona::Admin || k.tenant == tenant)
            .filter(|k| match glob {
                None => true,
                Some(pattern) => glob_match(pattern, &k.key),
            })
            .take(limit)
            .collect()
    }

    pub fn list_channels(&self, persona: ViewPersona, tenant: &str) -> Vec<&PubsubChannel> {
        self.channels
            .iter()
            .filter(|c| persona == ViewPersona::Admin || c.tenant == tenant)
            .collect()
    }

    pub fn cluster_slot_map(&self) -> ClusterSlotMap {
        let total_slots: u32 = self
            .slots
            .iter()
            .map(|s| (s.end - s.start + 1) as u32)
            .sum();
        ClusterSlotMap {
            ranges: self.slots.clone(),
            total_slots,
            coverage_pct: (total_slots as f64 / 16_384.0).clamp(0.0, 1.0),
        }
    }

    /// True when there is an unassigned slot in [0, 16383].
    pub fn has_unassigned_slots(&self) -> bool {
        let mut covered = vec![false; 16_384];
        for r in &self.slots {
            for i in r.start..=r.end {
                covered[i as usize] = true;
            }
        }
        covered.iter().any(|c| !c)
    }
}

// ── View-model panels ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardPanel {
    pub used_memory_bytes: u64,
    pub max_memory_bytes: u64,
    pub memory_utilisation: f64,
    pub hit_rate: f64,
    pub ops_per_sec: f64,
    pub evictions_per_sec: f64,
    pub fragmentation_ratio: f64,
    pub keys_total: u64,
    pub connected_clients: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterSlotMap {
    pub ranges: Vec<SlotRange>,
    pub total_slots: u32,
    /// Always in [0, 1].
    pub coverage_pct: f64,
}

// ── Validation + glob ────────────────────────────────────────────────────────

fn validate_key(key: &str) -> Result<(), CacheError> {
    if key.is_empty() {
        return Err(CacheError::InvalidKey("empty".into()));
    }
    if key.len() > 512 {
        return Err(CacheError::InvalidKey("> 512 bytes".into()));
    }
    if key.chars().any(|c| c.is_control()) {
        return Err(CacheError::InvalidKey("control char".into()));
    }
    Ok(())
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> ServerSnapshot {
        ServerSnapshot {
            used_memory_bytes: 100_000,
            used_memory_rss_bytes: 150_000,
            max_memory_bytes: 1_000_000,
            keys_total: 4_200,
            hits_per_sec: 800.0,
            lookups_per_sec: 1_000.0,
            ops_per_sec: 1_500.0,
            evictions_per_sec: 5.0,
            connected_clients: 42,
        }
    }

    #[test]
    fn snapshot_metrics_compute_correctly() {
        let s = snap();
        assert!((s.hit_rate() - 0.8).abs() < 1e-9);
        assert!((s.fragmentation_ratio() - 1.5).abs() < 1e-9);
        assert!((s.memory_utilisation() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn snapshot_handles_zero_lookups_and_zero_max_memory() {
        let mut s = snap();
        s.lookups_per_sec = 0.0;
        s.max_memory_bytes = 0;
        s.used_memory_bytes = 0;
        assert_eq!(s.hit_rate(), 0.0);
        assert_eq!(s.memory_utilisation(), 0.0);
        assert_eq!(s.fragmentation_ratio(), 0.0);
    }

    #[test]
    fn dashboard_reflects_snapshot() {
        let mut p = CachePlugin::new();
        p.set_snapshot(snap());
        let panel = p.dashboard();
        assert_eq!(panel.used_memory_bytes, 100_000);
        assert!((panel.hit_rate - 0.8).abs() < 1e-9);
        assert!(panel.fragmentation_ratio > 1.0);
        assert_eq!(panel.connected_clients, 42);
    }

    #[test]
    fn register_key_validates_input() {
        let mut p = CachePlugin::new();
        assert!(p
            .register_key(KeyEntry {
                key: "session:acme:abc".into(),
                tenant: "acme".into(),
                kind: KeyKind::String,
                size_bytes: 64,
                ttl_seconds: Some(300),
            })
            .is_ok());
        let err = p
            .register_key(KeyEntry {
                key: String::new(),
                tenant: "acme".into(),
                kind: KeyKind::String,
                size_bytes: 0,
                ttl_seconds: None,
            })
            .unwrap_err();
        assert!(matches!(err, CacheError::InvalidKey(_)));
    }

    #[test]
    fn list_keys_glob_filters_by_pattern() {
        let mut p = CachePlugin::new();
        for key in ["user:1", "user:2", "post:1"] {
            p.register_key(KeyEntry {
                key: key.into(),
                tenant: "acme".into(),
                kind: KeyKind::String,
                size_bytes: 16,
                ttl_seconds: None,
            })
            .unwrap();
        }
        assert_eq!(p.list_keys(Some("user:*"), ViewPersona::Admin, "acme", 100).len(), 2);
        assert_eq!(p.list_keys(Some("post:?"), ViewPersona::Admin, "acme", 100).len(), 1);
        assert_eq!(p.list_keys(None, ViewPersona::Admin, "acme", 100).len(), 3);
        assert_eq!(p.list_keys(None, ViewPersona::Admin, "acme", 1).len(), 1);
    }

    #[test]
    fn list_keys_scopes_to_tenant_for_non_admin() {
        let mut p = CachePlugin::new();
        p.register_key(KeyEntry {
            key: "session:acme:1".into(),
            tenant: "acme".into(),
            kind: KeyKind::String,
            size_bytes: 16,
            ttl_seconds: None,
        })
        .unwrap();
        p.register_key(KeyEntry {
            key: "session:globex:1".into(),
            tenant: "globex".into(),
            kind: KeyKind::String,
            size_bytes: 16,
            ttl_seconds: None,
        })
        .unwrap();
        assert_eq!(p.list_keys(None, ViewPersona::Tenant, "acme", 100).len(), 1);
        assert_eq!(p.list_keys(None, ViewPersona::Admin, "anything", 100).len(), 2);
    }

    #[test]
    fn slot_range_validates_bounds_and_overlap() {
        let mut p = CachePlugin::new();
        p.register_slot_range(SlotRange {
            start: 0,
            end: 5_460,
            primary_node: "n1".into(),
            replica_nodes: vec!["n1r".into()],
        })
        .unwrap();
        let err = p
            .register_slot_range(SlotRange {
                start: 100,
                end: 200,
                primary_node: "n2".into(),
                replica_nodes: vec![],
            })
            .unwrap_err();
        assert!(matches!(err, CacheError::OverlappingSlotRange(_, _)));
        let err = p
            .register_slot_range(SlotRange {
                start: 17_000,
                end: 17_500,
                primary_node: "n3".into(),
                replica_nodes: vec![],
            })
            .unwrap_err();
        assert!(matches!(err, CacheError::InvalidSlotRange(_, _)));
    }

    #[test]
    fn cluster_slot_map_computes_coverage() {
        let mut p = CachePlugin::new();
        p.register_slot_range(SlotRange {
            start: 0,
            end: 8_191,
            primary_node: "n1".into(),
            replica_nodes: vec![],
        })
        .unwrap();
        p.register_slot_range(SlotRange {
            start: 8_192,
            end: 16_383,
            primary_node: "n2".into(),
            replica_nodes: vec![],
        })
        .unwrap();
        let map = p.cluster_slot_map();
        assert_eq!(map.total_slots, 16_384);
        assert!((map.coverage_pct - 1.0).abs() < 1e-9);
        assert!(!p.has_unassigned_slots());
    }

    #[test]
    fn cluster_slot_map_detects_gaps() {
        let mut p = CachePlugin::new();
        p.register_slot_range(SlotRange {
            start: 0,
            end: 8_000,
            primary_node: "n1".into(),
            replica_nodes: vec![],
        })
        .unwrap();
        assert!(p.has_unassigned_slots());
        let map = p.cluster_slot_map();
        assert!(map.coverage_pct < 1.0);
    }

    #[test]
    fn list_channels_scopes_to_tenant() {
        let mut p = CachePlugin::new();
        p.register_channel(PubsubChannel {
            name: "events:acme".into(),
            tenant: "acme".into(),
            subscribers: 3,
            messages_per_sec: 100,
        });
        p.register_channel(PubsubChannel {
            name: "events:globex".into(),
            tenant: "globex".into(),
            subscribers: 1,
            messages_per_sec: 10,
        });
        assert_eq!(p.list_channels(ViewPersona::Tenant, "acme").len(), 1);
        assert_eq!(p.list_channels(ViewPersona::Admin, "acme").len(), 2);
    }
}

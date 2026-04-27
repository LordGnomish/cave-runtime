//! BPF map introspection — `cilium bpf <map> list` shape.
//!
//! Mirrors `pkg/maps/cmd/dump.go` and the per-map pretty-printers in
//! `pkg/maps/{ctmap,ipcache,policymap,...}/dump.go`. The agent exposes
//! a JSON dump endpoint per map; the `cilium bpf <kind> list` CLI
//! pretty-prints it.
//!
//! Each map has:
//!
//! * a `dump_kind` string ("ipcache", "policy", "ct_tcp_v4", "lb_v4",
//!   "endpoints", "snat_v4_external"),
//! * a per-key pretty-print format,
//! * fill metrics (count + capacity → fill ratio for /metrics).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BpfMapKind {
    Endpoints,
    Ipcache,
    Policy,
    CtTcp,
    CtAny,
    Nat,
    Lb,
    LbBackends,
    Lxc,
    Tunnel,
    EncryptKey,
    Auth,
    Sock,
    Other,
}

impl BpfMapKind {
    pub fn cli_name(self) -> &'static str {
        match self {
            BpfMapKind::Endpoints => "endpoints",
            BpfMapKind::Ipcache => "ipcache",
            BpfMapKind::Policy => "policy",
            BpfMapKind::CtTcp => "ct_tcp_v4",
            BpfMapKind::CtAny => "ct_any_v4",
            BpfMapKind::Nat => "snat_v4_external",
            BpfMapKind::Lb => "lb_v4",
            BpfMapKind::LbBackends => "lb_backends_v4",
            BpfMapKind::Lxc => "lxc",
            BpfMapKind::Tunnel => "tunnel",
            BpfMapKind::EncryptKey => "encrypt_key",
            BpfMapKind::Auth => "auth",
            BpfMapKind::Sock => "sock",
            BpfMapKind::Other => "other",
        }
    }
}

/// One entry in a dump — keys/values opaque so each per-map handler
/// can render them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BpfDumpEntry {
    pub key_pretty: String,
    pub value_pretty: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BpfMapDump {
    pub name: String,
    pub kind: BpfMapKind,
    pub max_entries: u64,
    pub entries: Vec<BpfDumpEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BpfFillMetric {
    pub used: u64,
    pub capacity: u64,
    pub ratio: f32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DumpError {
    #[error("map `{0}` not found")]
    NotFound(String),
    #[error("map `{0}` is at capacity ({1})")]
    AtCapacity(String, u64),
    #[error("tenant {tenant} cannot mutate dump registry owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct BpfMapRegistry {
    pub tenant: TenantId,
    maps: BTreeMap<String, BpfMapDump>,
}

impl BpfMapRegistry {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, maps: BTreeMap::new() }
    }

    pub fn register(&mut self, name: impl Into<String>, kind: BpfMapKind, max_entries: u64) {
        let name = name.into();
        self.maps.insert(name.clone(), BpfMapDump {
            name, kind, max_entries, entries: Vec::new(),
        });
    }

    pub fn unregister(&mut self, name: &str) -> Result<(), DumpError> {
        self.maps.remove(name).ok_or_else(|| DumpError::NotFound(name.to_string()))?;
        Ok(())
    }

    pub fn upsert_entry(&mut self, name: &str, entry: BpfDumpEntry) -> Result<(), DumpError> {
        let map = self.maps.get_mut(name).ok_or_else(|| DumpError::NotFound(name.to_string()))?;
        if let Some(slot) = map.entries.iter_mut().find(|e| e.key_pretty == entry.key_pretty) {
            *slot = entry;
            return Ok(());
        }
        if map.entries.len() as u64 >= map.max_entries {
            return Err(DumpError::AtCapacity(name.to_string(), map.max_entries));
        }
        map.entries.push(entry);
        Ok(())
    }

    pub fn remove_entry(&mut self, name: &str, key_pretty: &str) -> Result<(), DumpError> {
        let map = self.maps.get_mut(name).ok_or_else(|| DumpError::NotFound(name.to_string()))?;
        map.entries.retain(|e| e.key_pretty != key_pretty);
        Ok(())
    }

    pub fn dump(&self, name: &str) -> Result<&BpfMapDump, DumpError> {
        self.maps.get(name).ok_or_else(|| DumpError::NotFound(name.to_string()))
    }

    pub fn fill_metric(&self, name: &str) -> Result<BpfFillMetric, DumpError> {
        let map = self.maps.get(name).ok_or_else(|| DumpError::NotFound(name.to_string()))?;
        let used = map.entries.len() as u64;
        let cap = map.max_entries.max(1);
        Ok(BpfFillMetric {
            used, capacity: map.max_entries,
            ratio: (used as f32) / (cap as f32),
        })
    }

    pub fn list(&self) -> Vec<&BpfMapDump> {
        self.maps.values().collect()
    }

    pub fn count(&self) -> usize {
        self.maps.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/maps/cmd/dump.go", "Dump");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn reg(tenant: TenantId) -> BpfMapRegistry {
        BpfMapRegistry::new(tenant)
    }

    fn entry(k: &str, v: &str) -> BpfDumpEntry {
        BpfDumpEntry { key_pretty: k.into(), value_pretty: v.into() }
    }

    // ── BpfMapKind ─────────────────────────────────────────────────────────

    #[test]
    fn map_kind_cli_names() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Kind.CLIName", "tenant-bd-cn");
        assert_eq!(BpfMapKind::Endpoints.cli_name(), "endpoints");
        assert_eq!(BpfMapKind::Ipcache.cli_name(), "ipcache");
        assert_eq!(BpfMapKind::Policy.cli_name(), "policy");
        assert_eq!(BpfMapKind::CtTcp.cli_name(), "ct_tcp_v4");
        assert_eq!(BpfMapKind::CtAny.cli_name(), "ct_any_v4");
        assert_eq!(BpfMapKind::Nat.cli_name(), "snat_v4_external");
        assert_eq!(BpfMapKind::Lb.cli_name(), "lb_v4");
        assert_eq!(BpfMapKind::Tunnel.cli_name(), "tunnel");
    }

    // ── Registry ───────────────────────────────────────────────────────────

    #[test]
    fn registry_register_records_map() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Register", "tenant-bd-r");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 65536);
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn registry_unregister_drops_map() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Unregister", "tenant-bd-u");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 65536);
        r.unregister("cilium_ipcache").unwrap();
        assert_eq!(r.count(), 0);
    }

    #[test]
    fn registry_unregister_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Unregister.NotFound", "tenant-bd-unf");
        let mut r = reg(tenant);
        let err = r.unregister("ghost").unwrap_err();
        assert!(matches!(err, DumpError::NotFound(_)));
    }

    #[test]
    fn registry_register_replaces_in_place() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Register.Replace", "tenant-bd-rep");
        let mut r = reg(tenant);
        r.register("cilium_policy", BpfMapKind::Policy, 1024);
        r.register("cilium_policy", BpfMapKind::Policy, 4096);
        assert_eq!(r.count(), 1);
        assert_eq!(r.dump("cilium_policy").unwrap().max_entries, 4096);
    }

    // ── Entries ────────────────────────────────────────────────────────────

    #[test]
    fn upsert_entry_appends_new() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "UpsertEntry", "tenant-bd-ue");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 100);
        r.upsert_entry("cilium_ipcache", entry("10.0.0.1", "id=256")).unwrap();
        assert_eq!(r.dump("cilium_ipcache").unwrap().entries.len(), 1);
    }

    #[test]
    fn upsert_entry_replaces_existing_key() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "UpsertEntry.Replace", "tenant-bd-uer");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 100);
        r.upsert_entry("cilium_ipcache", entry("10.0.0.1", "id=256")).unwrap();
        r.upsert_entry("cilium_ipcache", entry("10.0.0.1", "id=999")).unwrap();
        let d = r.dump("cilium_ipcache").unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].value_pretty, "id=999");
    }

    #[test]
    fn upsert_entry_at_capacity_returns_error() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "UpsertEntry.AtCapacity", "tenant-bd-uec");
        let mut r = reg(tenant);
        r.register("cilium_lb", BpfMapKind::Lb, 2);
        r.upsert_entry("cilium_lb", entry("a", "x")).unwrap();
        r.upsert_entry("cilium_lb", entry("b", "y")).unwrap();
        let err = r.upsert_entry("cilium_lb", entry("c", "z")).unwrap_err();
        assert!(matches!(err, DumpError::AtCapacity(_, 2)));
    }

    #[test]
    fn upsert_entry_unknown_map_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "UpsertEntry.NotFound", "tenant-bd-uenf");
        let mut r = reg(tenant);
        let err = r.upsert_entry("ghost", entry("k", "v")).unwrap_err();
        assert!(matches!(err, DumpError::NotFound(_)));
    }

    #[test]
    fn remove_entry_drops_key() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "RemoveEntry", "tenant-bd-re");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 100);
        r.upsert_entry("cilium_ipcache", entry("10.0.0.1", "id=256")).unwrap();
        r.remove_entry("cilium_ipcache", "10.0.0.1").unwrap();
        assert_eq!(r.dump("cilium_ipcache").unwrap().entries.len(), 0);
    }

    #[test]
    fn remove_entry_no_match_is_a_noop() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "RemoveEntry.NoMatch", "tenant-bd-renm");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 100);
        r.upsert_entry("cilium_ipcache", entry("10.0.0.1", "id=256")).unwrap();
        r.remove_entry("cilium_ipcache", "no-such-key").unwrap();
        assert_eq!(r.dump("cilium_ipcache").unwrap().entries.len(), 1);
    }

    #[test]
    fn remove_entry_unknown_map_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "RemoveEntry.NotFound", "tenant-bd-renf");
        let mut r = reg(tenant);
        let err = r.remove_entry("ghost", "k").unwrap_err();
        assert!(matches!(err, DumpError::NotFound(_)));
    }

    // ── Dump ───────────────────────────────────────────────────────────────

    #[test]
    fn dump_returns_recorded_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Dump", "tenant-bd-d");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 100);
        r.upsert_entry("cilium_ipcache", entry("10.0.0.1", "id=256")).unwrap();
        r.upsert_entry("cilium_ipcache", entry("10.0.0.2", "id=257")).unwrap();
        let d = r.dump("cilium_ipcache").unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.kind, BpfMapKind::Ipcache);
    }

    #[test]
    fn dump_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Dump.NotFound", "tenant-bd-dnf");
        let r = reg(tenant);
        let err = r.dump("ghost").unwrap_err();
        assert!(matches!(err, DumpError::NotFound(_)));
    }

    #[test]
    fn list_returns_all_maps() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "List", "tenant-bd-l");
        let mut r = reg(tenant);
        r.register("cilium_ipcache", BpfMapKind::Ipcache, 100);
        r.register("cilium_policy", BpfMapKind::Policy, 100);
        r.register("cilium_ct_tcp4", BpfMapKind::CtTcp, 100);
        assert_eq!(r.list().len(), 3);
    }

    // ── Fill metrics ───────────────────────────────────────────────────────

    #[test]
    fn fill_metric_reports_used_capacity_ratio() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "FillMetric", "tenant-bd-fm");
        let mut r = reg(tenant);
        r.register("cilium_policy", BpfMapKind::Policy, 100);
        for i in 0..25 {
            r.upsert_entry("cilium_policy", entry(&format!("k{i}"), "v")).unwrap();
        }
        let m = r.fill_metric("cilium_policy").unwrap();
        assert_eq!(m.used, 25);
        assert_eq!(m.capacity, 100);
        assert!((m.ratio - 0.25).abs() < 1e-6);
    }

    #[test]
    fn fill_metric_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "FillMetric.NotFound", "tenant-bd-fmnf");
        let r = reg(tenant);
        let err = r.fill_metric("ghost").unwrap_err();
        assert!(matches!(err, DumpError::NotFound(_)));
    }

    #[test]
    fn fill_metric_zero_capacity_does_not_divide_by_zero() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "FillMetric.ZeroCap", "tenant-bd-fmzc");
        let mut r = reg(tenant);
        r.register("zero", BpfMapKind::Other, 0);
        let m = r.fill_metric("zero").unwrap();
        assert_eq!(m.used, 0);
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn map_kind_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Kind.Serde", "tenant-bd-kserde");
        for k in [
            BpfMapKind::Endpoints, BpfMapKind::Ipcache, BpfMapKind::Policy,
            BpfMapKind::CtTcp, BpfMapKind::Lb, BpfMapKind::Auth,
        ] {
            let s = serde_json::to_string(&k).unwrap();
            let back: BpfMapKind = serde_json::from_str(&s).unwrap();
            assert_eq!(back, k);
        }
    }

    #[test]
    fn dump_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "Dump.Serde", "tenant-bd-dserde");
        let mut r = reg(tenant);
        r.register("cilium_policy", BpfMapKind::Policy, 100);
        r.upsert_entry("cilium_policy", entry("k", "v")).unwrap();
        let d = r.dump("cilium_policy").unwrap();
        let json = serde_json::to_string(d).unwrap();
        let back: BpfMapDump = serde_json::from_str(&json).unwrap();
        assert_eq!(back, *d);
    }

    #[test]
    fn fill_metric_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/cmd/dump.go", "FillMetric.Serde", "tenant-bd-fmserde");
        let m = BpfFillMetric { used: 50, capacity: 100, ratio: 0.5 };
        let s = serde_json::to_string(&m).unwrap();
        let back: BpfFillMetric = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }
}

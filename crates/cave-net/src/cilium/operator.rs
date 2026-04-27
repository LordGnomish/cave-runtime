//! cilium-operator workflows — identity garbage collection, CES batching,
//! stale CRD cleanup.
//!
//! Mirrors:
//!
//! * `pkg/operator/identitygc/identitygc.go` — periodic sweep that
//!   deletes Cilium identities no longer referenced by any endpoint.
//! * `pkg/operator/pkg/ciliumendpointslice/manager.go` — the CES
//!   batcher that packs CiliumEndpoint observations into slices of
//!   size `≤ ces_max_size` (default 100).
//! * `pkg/operator/pkg/ciliumendpoint/cleanup.go` — stale-endpoint
//!   reaper that removes CiliumEndpoint CRs whose pod is gone.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

// ── Identity GC ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRefCount {
    pub identity: u32,
    pub references: u32,
    pub last_observed_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityGcReport {
    pub scanned: u64,
    pub deleted: u64,
    pub retained: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OperatorError {
    #[error("identity {0} is below the minimum allocatable range")]
    ReservedIdentity(u32),
    #[error("CES `{0}` already exists")]
    CesAlreadyExists(String),
    #[error("CES `{0}` not found")]
    CesNotFound(String),
    #[error("CiliumEndpoint `{0}` not found")]
    CiliumEndpointNotFound(String),
    #[error("tenant {tenant} cannot mutate operator state owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct IdentityGc {
    pub tenant: TenantId,
    pub min_identity: u32,
    /// Grace period before an unreferenced identity is GC'd.
    pub grace_seconds: u64,
    refcounts: HashMap<u32, IdentityRefCount>,
}

impl IdentityGc {
    pub fn new(tenant: TenantId, min_identity: u32, grace_seconds: u64) -> Self {
        Self { tenant, min_identity, grace_seconds, refcounts: HashMap::new() }
    }

    pub fn add_reference(&mut self, identity: u32, now_ns: u64) -> Result<(), OperatorError> {
        if identity < self.min_identity {
            return Err(OperatorError::ReservedIdentity(identity));
        }
        let entry = self.refcounts.entry(identity).or_insert(IdentityRefCount {
            identity, references: 0, last_observed_ns: now_ns,
        });
        entry.references += 1;
        entry.last_observed_ns = now_ns;
        Ok(())
    }

    pub fn release_reference(&mut self, identity: u32, now_ns: u64) -> Result<(), OperatorError> {
        if identity < self.min_identity {
            return Err(OperatorError::ReservedIdentity(identity));
        }
        if let Some(e) = self.refcounts.get_mut(&identity) {
            if e.references > 0 {
                e.references -= 1;
            }
            e.last_observed_ns = now_ns;
        }
        Ok(())
    }

    pub fn ref_count(&self, identity: u32) -> u32 {
        self.refcounts.get(&identity).map(|e| e.references).unwrap_or(0)
    }

    /// Run a sweep: delete identities with `references == 0` and
    /// `last_observed + grace_seconds * 1e9 ≤ now_ns`.
    pub fn sweep(&mut self, now_ns: u64) -> IdentityGcReport {
        let grace_ns = self.grace_seconds * 1_000_000_000;
        let mut report = IdentityGcReport { scanned: 0, deleted: 0, retained: 0 };
        let stale: Vec<u32> = self.refcounts.iter()
            .filter(|(_, e)| {
                report.scanned += 1;
                if e.references > 0 {
                    report.retained += 1;
                    return false;
                }
                let elapsed = now_ns.saturating_sub(e.last_observed_ns);
                let dead = elapsed >= grace_ns;
                if !dead {
                    report.retained += 1;
                }
                dead
            })
            .map(|(k, _)| *k)
            .collect();
        for k in &stale {
            self.refcounts.remove(k);
            report.deleted += 1;
        }
        report
    }

    pub fn tracked_count(&self) -> usize {
        self.refcounts.len()
    }
}

// ── CES batching ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiliumEndpoint {
    pub name: String,
    pub namespace: String,
    pub identity: u32,
    pub pod_name: String,
}

impl CiliumEndpoint {
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiliumEndpointSlice {
    pub name: String,
    pub endpoints: Vec<CiliumEndpoint>,
}

#[derive(Debug)]
pub struct CesManager {
    pub tenant: TenantId,
    pub max_size: usize,
    slices: BTreeMap<String, CiliumEndpointSlice>,
    /// Reverse: endpoint key → CES name for fast lookup on update/delete.
    endpoint_to_ces: HashMap<String, String>,
    next_id: u64,
}

impl CesManager {
    pub fn new(tenant: TenantId, max_size: usize) -> Self {
        Self {
            tenant, max_size,
            slices: BTreeMap::new(),
            endpoint_to_ces: HashMap::new(),
            next_id: 1,
        }
    }

    /// Insert or update a CiliumEndpoint, packing it into an existing
    /// non-full CES or creating a new one. Returns the CES name.
    pub fn upsert(&mut self, ep: CiliumEndpoint) -> String {
        let key = ep.key();
        // If this endpoint already lives in a CES, update in place.
        if let Some(ces_name) = self.endpoint_to_ces.get(&key).cloned() {
            if let Some(s) = self.slices.get_mut(&ces_name) {
                if let Some(slot) = s.endpoints.iter_mut().find(|e| e.key() == key) {
                    *slot = ep;
                    return ces_name;
                }
            }
        }
        // Find an existing CES with capacity.
        for s in self.slices.values_mut() {
            if s.endpoints.len() < self.max_size {
                s.endpoints.push(ep.clone());
                let name = s.name.clone();
                self.endpoint_to_ces.insert(key, name.clone());
                return name;
            }
        }
        // Make a new CES.
        let name = format!("ces-{}", self.next_id);
        self.next_id += 1;
        let s = CiliumEndpointSlice { name: name.clone(), endpoints: vec![ep] };
        self.slices.insert(name.clone(), s);
        self.endpoint_to_ces.insert(key, name.clone());
        name
    }

    pub fn remove(&mut self, ep_key: &str) -> Result<(), OperatorError> {
        let ces_name = self.endpoint_to_ces.remove(ep_key)
            .ok_or_else(|| OperatorError::CiliumEndpointNotFound(ep_key.to_string()))?;
        let drop_ces = if let Some(s) = self.slices.get_mut(&ces_name) {
            s.endpoints.retain(|e| e.key() != ep_key);
            s.endpoints.is_empty()
        } else {
            false
        };
        if drop_ces {
            self.slices.remove(&ces_name);
        }
        Ok(())
    }

    pub fn ces_count(&self) -> usize {
        self.slices.len()
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoint_to_ces.len()
    }

    pub fn ces(&self, name: &str) -> Option<&CiliumEndpointSlice> {
        self.slices.get(name)
    }

    pub fn ces_for_endpoint(&self, ep_key: &str) -> Option<&str> {
        self.endpoint_to_ces.get(ep_key).map(|s| s.as_str())
    }
}

// ── Stale CR cleanup ────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct StaleCrCleanup {
    pub tenant: TenantId,
}

impl StaleCrCleanup {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant }
    }

    /// Compute the set of CR names to delete: every CR in `crs` whose
    /// associated pod key is *not* in `live_pods`. Mirrors
    /// `pkg/operator/pkg/ciliumendpoint/cleanup.go::ReconcileEndpoints`.
    pub fn stale_endpoints<'a>(&self, crs: &'a [(String, String)], live_pods: &HashSet<String>) -> Vec<&'a str> {
        crs.iter()
            .filter(|(_, pod_key)| !live_pods.contains(pod_key))
            .map(|(cr_name, _)| cr_name.as_str())
            .collect()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/operator/identitygc/identitygc.go", "GarbageCollector");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn endpoint(ns: &str, name: &str, identity: u32) -> CiliumEndpoint {
        CiliumEndpoint {
            name: name.into(), namespace: ns.into(),
            identity, pod_name: name.into(),
        }
    }

    // ── IdentityGc ──────────────────────────────────────────────────────────

    #[test]
    fn idgc_add_reference_increments_count() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "AddRef", "tenant-op-add");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        gc.add_reference(256, 100).unwrap();
        gc.add_reference(256, 100).unwrap();
        assert_eq!(gc.ref_count(256), 2);
    }

    #[test]
    fn idgc_release_reference_decrements_count() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "ReleaseRef", "tenant-op-rel");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        gc.add_reference(256, 100).unwrap();
        gc.add_reference(256, 100).unwrap();
        gc.release_reference(256, 100).unwrap();
        assert_eq!(gc.ref_count(256), 1);
    }

    #[test]
    fn idgc_release_below_zero_clamps_to_zero() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "Release.Clamp", "tenant-op-clamp");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        gc.add_reference(256, 100).unwrap();
        gc.release_reference(256, 100).unwrap();
        gc.release_reference(256, 100).unwrap(); // already zero
        assert_eq!(gc.ref_count(256), 0);
    }

    #[test]
    fn idgc_reserved_identity_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "AddRef.Reserved", "tenant-op-res");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        let err = gc.add_reference(1 /* reserved:host */, 100).unwrap_err();
        assert_eq!(err, OperatorError::ReservedIdentity(1));
    }

    #[test]
    fn idgc_sweep_deletes_unreferenced_after_grace() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "Sweep", "tenant-op-sweep");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        gc.add_reference(256, 0).unwrap();
        gc.release_reference(256, 0).unwrap();
        let report = gc.sweep(60_000_000_000 + 1);
        assert_eq!(report.deleted, 1);
        assert_eq!(gc.tracked_count(), 0);
    }

    #[test]
    fn idgc_sweep_keeps_referenced() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "Sweep.KeepRef", "tenant-op-skp");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        gc.add_reference(256, 0).unwrap();
        let report = gc.sweep(60_000_000_000 + 1);
        assert_eq!(report.deleted, 0);
        assert_eq!(report.retained, 1);
    }

    #[test]
    fn idgc_sweep_keeps_recently_unreferenced_within_grace() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "Sweep.WithinGrace", "tenant-op-grace");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        gc.add_reference(256, 0).unwrap();
        gc.release_reference(256, 0).unwrap();
        let report = gc.sweep(30_000_000_000); // 30s, before grace
        assert_eq!(report.deleted, 0);
        assert_eq!(report.retained, 1);
    }

    #[test]
    fn idgc_sweep_report_counts_all_categories() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "Sweep.Report", "tenant-op-rep");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        gc.add_reference(256, 0).unwrap();
        gc.add_reference(257, 0).unwrap();
        gc.release_reference(257, 0).unwrap();
        let report = gc.sweep(60_000_000_000 + 1);
        assert_eq!(report.scanned, 2);
        assert_eq!(report.retained, 1);
        assert_eq!(report.deleted, 1);
    }

    #[test]
    fn idgc_release_reserved_returns_error() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "Release.Reserved", "tenant-op-relres");
        let mut gc = IdentityGc::new(tenant, 256, 60);
        let err = gc.release_reference(1, 100).unwrap_err();
        assert_eq!(err, OperatorError::ReservedIdentity(1));
    }

    // ── CES batching ────────────────────────────────────────────────────────

    #[test]
    fn ces_first_endpoint_creates_first_ces() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Upsert.First", "tenant-op-c1");
        let mut m = CesManager::new(tenant, 100);
        let n = m.upsert(endpoint("ns", "p1", 256));
        assert_eq!(n, "ces-1");
        assert_eq!(m.ces_count(), 1);
    }

    #[test]
    fn ces_packs_multiple_endpoints_into_one_slice() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Upsert.Pack", "tenant-op-pack");
        let mut m = CesManager::new(tenant, 100);
        for i in 0..50u32 {
            m.upsert(endpoint("ns", &format!("p{i}"), 256 + i));
        }
        assert_eq!(m.ces_count(), 1);
    }

    #[test]
    fn ces_creates_new_slice_when_full() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Upsert.NewSlice", "tenant-op-new");
        let mut m = CesManager::new(tenant, 3);
        for i in 0..7u32 {
            m.upsert(endpoint("ns", &format!("p{i}"), 256 + i));
        }
        assert_eq!(m.ces_count(), 3);
    }

    #[test]
    fn ces_remove_drops_endpoint_and_collapses_empty_slice() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Remove", "tenant-op-rmep");
        let mut m = CesManager::new(tenant, 100);
        m.upsert(endpoint("ns", "p1", 256));
        m.remove("ns/p1").unwrap();
        assert_eq!(m.endpoint_count(), 0);
        assert_eq!(m.ces_count(), 0);
    }

    #[test]
    fn ces_remove_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Remove.NotFound", "tenant-op-rmnf");
        let mut m = CesManager::new(tenant, 100);
        let err = m.remove("ns/ghost").unwrap_err();
        assert!(matches!(err, OperatorError::CiliumEndpointNotFound(_)));
    }

    #[test]
    fn ces_upsert_replaces_existing_endpoint_in_place() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Upsert.Replace", "tenant-op-upr");
        let mut m = CesManager::new(tenant, 100);
        m.upsert(endpoint("ns", "p1", 256));
        m.upsert(endpoint("ns", "p1", 999));
        assert_eq!(m.endpoint_count(), 1);
        let ces_name = m.ces_for_endpoint("ns/p1").unwrap().to_string();
        let ces = m.ces(&ces_name).unwrap();
        assert_eq!(ces.endpoints[0].identity, 999);
    }

    #[test]
    fn ces_lookup_for_endpoint_returns_owning_ces() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Lookup", "tenant-op-lk");
        let mut m = CesManager::new(tenant, 100);
        let n = m.upsert(endpoint("ns", "p1", 256));
        assert_eq!(m.ces_for_endpoint("ns/p1"), Some(n.as_str()));
    }

    #[test]
    fn ces_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Lookup.NotFound", "tenant-op-lknf");
        let m = CesManager::new(tenant, 100);
        assert!(m.ces_for_endpoint("ns/ghost").is_none());
    }

    #[test]
    fn ces_max_size_respected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "MaxSize", "tenant-op-max");
        let mut m = CesManager::new(tenant, 5);
        for i in 0..10u32 {
            m.upsert(endpoint("ns", &format!("p{i}"), 256 + i));
        }
        for s in m.slices.values() {
            assert!(s.endpoints.len() <= 5);
        }
    }

    #[test]
    fn ces_endpoint_count_tracks_upserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Count", "tenant-op-cnt");
        let mut m = CesManager::new(tenant, 100);
        for i in 0..15u32 {
            m.upsert(endpoint("ns", &format!("p{i}"), 256 + i));
        }
        assert_eq!(m.endpoint_count(), 15);
    }

    #[test]
    fn ces_remove_some_keeps_slice_alive() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Remove.PartialSlice", "tenant-op-prt");
        let mut m = CesManager::new(tenant, 100);
        for i in 0..3u32 {
            m.upsert(endpoint("ns", &format!("p{i}"), 256 + i));
        }
        m.remove("ns/p0").unwrap();
        assert_eq!(m.endpoint_count(), 2);
        assert_eq!(m.ces_count(), 1);
    }

    // ── Stale CR cleanup ────────────────────────────────────────────────────

    #[test]
    fn stale_cleanup_returns_crs_without_live_pod() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpoint/cleanup.go", "Stale.Detect", "tenant-op-stale");
        let s = StaleCrCleanup::new(tenant);
        let crs = vec![
            ("cep-a".into(), "ns/p1".into()),
            ("cep-b".into(), "ns/p2".into()),
            ("cep-c".into(), "ns/p3".into()),
        ];
        let mut live: HashSet<String> = HashSet::new();
        live.insert("ns/p1".into());
        live.insert("ns/p3".into());
        let stale = s.stale_endpoints(&crs, &live);
        assert_eq!(stale, vec!["cep-b"]);
    }

    #[test]
    fn stale_cleanup_empty_input_returns_empty() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpoint/cleanup.go", "Stale.Empty", "tenant-op-empt");
        let s = StaleCrCleanup::new(tenant);
        let stale = s.stale_endpoints(&[], &HashSet::new());
        assert!(stale.is_empty());
    }

    #[test]
    fn stale_cleanup_all_live_returns_empty() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpoint/cleanup.go", "Stale.AllLive", "tenant-op-all");
        let s = StaleCrCleanup::new(tenant);
        let crs = vec![
            ("cep-a".into(), "ns/p1".into()),
            ("cep-b".into(), "ns/p2".into()),
        ];
        let mut live: HashSet<String> = HashSet::new();
        live.insert("ns/p1".into());
        live.insert("ns/p2".into());
        let stale = s.stale_endpoints(&crs, &live);
        assert!(stale.is_empty());
    }

    #[test]
    fn stale_cleanup_all_stale_returns_all() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpoint/cleanup.go", "Stale.AllStale", "tenant-op-allstl");
        let s = StaleCrCleanup::new(tenant);
        let crs = vec![
            ("cep-a".into(), "ns/p1".into()),
            ("cep-b".into(), "ns/p2".into()),
        ];
        let stale = s.stale_endpoints(&crs, &HashSet::new());
        assert_eq!(stale.len(), 2);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn idgc_report_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/operator/identitygc/identitygc.go", "Report.Serde", "tenant-op-rserde");
        let r = IdentityGcReport { scanned: 100, deleted: 5, retained: 95 };
        let s = serde_json::to_string(&r).unwrap();
        let back: IdentityGcReport = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn ces_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpointslice/manager.go", "Slice.Serde", "tenant-op-cserde");
        let ces = CiliumEndpointSlice {
            name: "ces-1".into(),
            endpoints: vec![endpoint("ns", "p1", 256)],
        };
        let s = serde_json::to_string(&ces).unwrap();
        let back: CiliumEndpointSlice = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ces);
    }

    #[test]
    fn cilium_endpoint_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/operator/pkg/ciliumendpoint/types.go", "CEP.Serde", "tenant-op-eserde");
        let e = endpoint("ns", "p1", 256);
        let s = serde_json::to_string(&e).unwrap();
        let back: CiliumEndpoint = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    // ── Integration: GC + CES ───────────────────────────────────────────────

    #[test]
    fn idgc_and_ces_independent_lifecycles() {
        let (_c, tenant) = cilium_test_ctx!("pkg/operator", "Integration", "tenant-op-int");
        let mut gc = IdentityGc::new(tenant.clone(), 256, 60);
        let mut ces = CesManager::new(tenant, 100);
        for i in 0..3u32 {
            ces.upsert(endpoint("ns", &format!("p{i}"), 256 + i));
            gc.add_reference(256 + i, 0).unwrap();
        }
        assert_eq!(gc.tracked_count(), 3);
        assert_eq!(ces.endpoint_count(), 3);
        // Pod p0 deleted: drop CES entry and release identity ref.
        ces.remove("ns/p0").unwrap();
        gc.release_reference(256, 0).unwrap();
        let report = gc.sweep(60_000_000_000 + 1);
        assert_eq!(report.deleted, 1);
        assert_eq!(gc.tracked_count(), 2);
    }
}

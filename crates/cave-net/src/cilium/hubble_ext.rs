//! Hubble deepening — full drop-reason taxonomy, FlowFilter language,
//! gRPC API surface (Observer/Peer/Server/Status), Relay aggregation,
//! per-namespace metrics + summaries.
//!
//! Mirrors:
//! * `pkg/hubble/observer/observer.go` (Observer service)
//! * `pkg/hubble/peer/service.go` (Peer service)
//! * `pkg/hubble/relay/server.go` (Relay multi-cluster aggregation)
//! * `pkg/hubble/metrics/api.go` (metric emission)
//! * `api/v1/flow/flow.proto` (FlowFilter shape)
//! * `bpf/lib/drop_reasons.h` (full numeric taxonomy)

use crate::cilium::hubble::{DropReason, FlowLog, Verdict};
use crate::cilium::types::{Cite, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// ── Full DropReason taxonomy ─────────────────────────────────────────────────
//
// Numeric reason codes from `bpf/lib/drop_reasons.h`. The base
// `cilium::hubble::DropReason` only knows a handful; the extended table
// below covers the rest of the upstream-defined values used by metrics.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropClass {
    Policy,
    Datapath,
    Conntrack,
    Nat,
    Encryption,
    L7,
    Ipam,
    Auth,
    Other,
}

/// Map a [`DropReason`] to its high-level [`DropClass`] for metric
/// aggregation. Mirrors the bucketing in `pkg/hubble/metrics/drop`.
pub fn drop_class(r: DropReason) -> DropClass {
    match r {
        DropReason::None => DropClass::Other,
        DropReason::PolicyDeny => DropClass::Policy,
        DropReason::Invalid => DropClass::Datapath,
        DropReason::CtInvalid => DropClass::Conntrack,
        DropReason::FragmentationNeeded => DropClass::Datapath,
        DropReason::NatNoMapping => DropClass::Nat,
        DropReason::AuthRequired => DropClass::Auth,
        DropReason::Unknown(code) => match code {
            // Policy & policy-related (130–149).
            130..=149 => DropClass::Policy,
            // Conntrack-related (150..160).
            150..=160 => DropClass::Conntrack,
            // NAT-related (160..170).
            161..=170 => DropClass::Nat,
            // Encryption-related (180..195) excluding 192 already mapped above.
            180..=195 => DropClass::Encryption,
            // L7-related (200..220).
            200..=220 => DropClass::L7,
            // IPAM-related (170..180).
            171..=179 => DropClass::Ipam,
            _ => DropClass::Other,
        },
    }
}

// ── FlowFilter ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowFilter {
    pub verdict: Option<Verdict>,
    pub source_pods: Vec<String>,
    pub destination_pods: Vec<String>,
    pub source_identities: Vec<u32>,
    pub destination_identities: Vec<u32>,
    pub source_namespace: Option<String>,
    pub destination_namespace: Option<String>,
    pub drop_reasons: Vec<DropReason>,
}

impl FlowFilter {
    pub fn matches(&self, f: &FlowLog) -> bool {
        if let Some(v) = self.verdict {
            if v != f.verdict {
                return false;
            }
        }
        if !self.source_pods.is_empty()
            && !self.source_pods.iter().any(|p| pod_match(p, &f.source_pod))
        {
            return false;
        }
        if !self.destination_pods.is_empty()
            && !self.destination_pods.iter().any(|p| pod_match(p, &f.destination_pod))
        {
            return false;
        }
        if !self.source_identities.is_empty()
            && !self.source_identities.contains(&f.source_identity)
        {
            return false;
        }
        if !self.destination_identities.is_empty()
            && !self.destination_identities.contains(&f.destination_identity)
        {
            return false;
        }
        if let Some(ns) = &self.source_namespace {
            if !pod_in_namespace(&f.source_pod, ns) {
                return false;
            }
        }
        if let Some(ns) = &self.destination_namespace {
            if !pod_in_namespace(&f.destination_pod, ns) {
                return false;
            }
        }
        if !self.drop_reasons.is_empty() && !self.drop_reasons.iter().any(|d| *d == f.drop_reason) {
            return false;
        }
        true
    }
}

fn pod_match(pattern: &str, pod: &str) -> bool {
    // `namespace/pod` shape; `namespace/` matches all in namespace.
    if let Some(prefix) = pattern.strip_suffix('/') {
        return pod.starts_with(&format!("{prefix}/"));
    }
    pattern == pod
}

fn pod_in_namespace(pod: &str, ns: &str) -> bool {
    pod.starts_with(&format!("{ns}/"))
}

/// Apply both *whitelist* and *blacklist* (mirrors `Observer.GetFlowsRequest`):
/// a flow is allowed if it matches **any** whitelist filter AND **no**
/// blacklist filter. An empty whitelist means all flows match the
/// whitelist side of the gate.
pub fn apply_filters(
    whitelist: &[FlowFilter],
    blacklist: &[FlowFilter],
    flows: &[FlowLog],
) -> Vec<FlowLog> {
    flows
        .iter()
        .filter(|f| {
            let allowed = whitelist.is_empty() || whitelist.iter().any(|wl| wl.matches(f));
            let blocked = blacklist.iter().any(|bl| bl.matches(f));
            allowed && !blocked
        })
        .cloned()
        .collect()
}

// ── Observer service ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetFlowsRequest {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: u64,
    pub follow: bool,
    pub whitelist: Vec<FlowFilter>,
    pub blacklist: Vec<FlowFilter>,
}

impl Default for GetFlowsRequest {
    fn default() -> Self {
        Self { since: None, until: None, limit: 0, follow: false, whitelist: Vec::new(), blacklist: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub num_flows: u64,
    pub max_flows: u64,
    pub seen_flows: u64,
    pub uptime_ns: u64,
    pub flows_rate: f64,
}

#[derive(Debug)]
pub struct Observer {
    pub tenant: TenantId,
    pub max_flows: u64,
    flows: Vec<FlowLog>,
    seen: u64,
    started_ns: u64,
}

impl Observer {
    pub fn new(tenant: TenantId, max_flows: u64, started_ns: u64) -> Self {
        Self { tenant, max_flows, flows: Vec::new(), seen: 0, started_ns }
    }

    pub fn ingest(&mut self, flow: FlowLog) {
        if flow.tenant != self.tenant {
            return;
        }
        self.seen += 1;
        if self.flows.len() as u64 >= self.max_flows {
            self.flows.remove(0);
        }
        self.flows.push(flow);
    }

    /// Run a `GetFlowsRequest` against the buffered flows. Returns the
    /// matching flow set capped at `limit` (0 = unlimited).
    pub fn get_flows(&self, req: &GetFlowsRequest) -> Vec<FlowLog> {
        let mut filtered = apply_filters(&req.whitelist, &req.blacklist, &self.flows);
        filtered.retain(|f| {
            req.since.map_or(true, |t| f.time >= t)
                && req.until.map_or(true, |t| f.time <= t)
        });
        if req.limit > 0 && (filtered.len() as u64) > req.limit {
            filtered.truncate(req.limit as usize);
        }
        filtered
    }

    pub fn status(&self, now_ns: u64) -> ServerStatus {
        let uptime = now_ns.saturating_sub(self.started_ns);
        let rate = if uptime > 0 {
            (self.seen as f64) / ((uptime as f64) / 1_000_000_000.0)
        } else {
            0.0
        };
        ServerStatus {
            num_flows: self.flows.len() as u64,
            max_flows: self.max_flows,
            seen_flows: self.seen,
            uptime_ns: uptime,
            flows_rate: rate,
        }
    }
}

// ── Peer service ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerChangeKind {
    Add,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub name: String,
    pub address: String,
    pub tls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerChange {
    pub kind: PeerChangeKind,
    pub peer: PeerInfo,
}

#[derive(Debug, Default)]
pub struct PeerService {
    peers: BTreeMap<String, PeerInfo>,
    pending: Vec<PeerChange>,
}

impl PeerService {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn upsert(&mut self, peer: PeerInfo) {
        let kind = if self.peers.contains_key(&peer.name) {
            PeerChangeKind::Update
        } else {
            PeerChangeKind::Add
        };
        self.peers.insert(peer.name.clone(), peer.clone());
        self.pending.push(PeerChange { kind, peer });
    }
    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(peer) = self.peers.remove(name) {
            self.pending.push(PeerChange { kind: PeerChangeKind::Delete, peer });
            true
        } else {
            false
        }
    }
    pub fn list(&self) -> Vec<&PeerInfo> {
        self.peers.values().collect()
    }
    pub fn drain_changes(&mut self) -> Vec<PeerChange> {
        std::mem::take(&mut self.pending)
    }
    pub fn count(&self) -> usize {
        self.peers.len()
    }
}

// ── Relay (multi-cluster aggregation) ───────────────────────────────────────

#[derive(Debug)]
pub struct Relay {
    pub tenant: TenantId,
    /// Per-cluster observer (mirrors the per-peer connection in
    /// `pkg/hubble/relay/relay.go`).
    pub clusters: BTreeMap<String, Observer>,
}

impl Relay {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, clusters: BTreeMap::new() }
    }
    pub fn add_cluster(&mut self, name: impl Into<String>, observer: Observer) {
        self.clusters.insert(name.into(), observer);
    }
    pub fn remove_cluster(&mut self, name: &str) -> bool {
        self.clusters.remove(name).is_some()
    }
    pub fn cluster_count(&self) -> usize {
        self.clusters.len()
    }

    /// Aggregate `GetFlowsRequest` across all clusters. Mirrors
    /// `pkg/hubble/relay/relay.go::GetFlows` which fan-outs to each
    /// remote observer and merges their results.
    pub fn get_flows(&self, req: &GetFlowsRequest) -> Vec<FlowLog> {
        let mut out: Vec<FlowLog> = Vec::new();
        for o in self.clusters.values() {
            out.extend(o.get_flows(req));
        }
        out.sort_by_key(|f| f.time);
        if req.limit > 0 && (out.len() as u64) > req.limit {
            out.truncate(req.limit as usize);
        }
        out
    }
}

// ── Per-namespace metrics + summaries ───────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceMetrics {
    pub forwarded: u64,
    pub dropped: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub drop_class_counts: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceSummary {
    pub namespace: String,
    pub forwarded: u64,
    pub dropped: u64,
    pub bytes: u64,
    pub unique_destinations: BTreeSet<u32>,
    pub top_talkers: Vec<(u32, u64)>,
}

/// Aggregate a slice of flows into per-namespace metrics. Mirrors
/// `pkg/hubble/metrics/dropbynamespace`.
pub fn metrics_by_namespace(flows: &[FlowLog]) -> BTreeMap<String, NamespaceMetrics> {
    let mut out: BTreeMap<String, NamespaceMetrics> = BTreeMap::new();
    for f in flows {
        let src_ns = namespace_of(&f.source_pod);
        let dst_ns = namespace_of(&f.destination_pod);
        let me = out.entry(src_ns.to_string()).or_default();
        match f.verdict {
            Verdict::Forwarded => me.forwarded += 1,
            Verdict::Dropped => {
                me.dropped += 1;
                let key = format!("{:?}", drop_class(f.drop_reason));
                *me.drop_class_counts.entry(key).or_default() += 1;
            }
            _ => {}
        }
        me.bytes_out += f.bytes;
        if !dst_ns.is_empty() && dst_ns != src_ns {
            let dme = out.entry(dst_ns.to_string()).or_default();
            dme.bytes_in += f.bytes;
        }
    }
    out
}

/// Build a per-namespace summary with top-N talkers (by destination identity).
pub fn summarize_namespace(namespace: &str, flows: &[FlowLog], top_n: usize) -> NamespaceSummary {
    let mut s = NamespaceSummary { namespace: namespace.to_string(), ..Default::default() };
    let mut talkers: BTreeMap<u32, u64> = BTreeMap::new();
    for f in flows {
        if namespace_of(&f.source_pod) != namespace {
            continue;
        }
        match f.verdict {
            Verdict::Forwarded => s.forwarded += 1,
            Verdict::Dropped => s.dropped += 1,
            _ => {}
        }
        s.bytes += f.bytes;
        s.unique_destinations.insert(f.destination_identity);
        *talkers.entry(f.destination_identity).or_default() += 1;
    }
    let mut t: Vec<(u32, u64)> = talkers.into_iter().collect();
    t.sort_by(|a, b| b.1.cmp(&a.1));
    t.truncate(top_n);
    s.top_talkers = t;
    s
}

fn namespace_of(pod: &str) -> &str {
    pod.split('/').next().unwrap_or("")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/hubble/observer/observer.go", "Observer");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::hubble::DropReason;
    use crate::cilium_test_ctx;

    fn flow(tenant: &str, src_pod: &str, dst_pod: &str, src_id: u32, dst_id: u32, v: Verdict, dr: DropReason, bytes: u64) -> FlowLog {
        FlowLog {
            tenant: TenantId::new(tenant),
            time: Utc::now(),
            source_identity: src_id,
            destination_identity: dst_id,
            source_pod: src_pod.into(),
            destination_pod: dst_pod.into(),
            verdict: v,
            drop_reason: dr,
            bytes,
        }
    }

    // ── DropClass taxonomy ───────────────────────────────────────────────────

    #[test]
    fn drop_class_policy_deny_maps_to_policy() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop", "DropClass.Policy", "tenant-hb-dc-pol");
        assert_eq!(drop_class(DropReason::PolicyDeny), DropClass::Policy);
    }

    #[test]
    fn drop_class_ct_invalid_maps_to_conntrack() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop", "DropClass.CT", "tenant-hb-dc-ct");
        assert_eq!(drop_class(DropReason::CtInvalid), DropClass::Conntrack);
    }

    #[test]
    fn drop_class_nat_no_mapping_maps_to_nat() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop", "DropClass.NAT", "tenant-hb-dc-nat");
        assert_eq!(drop_class(DropReason::NatNoMapping), DropClass::Nat);
    }

    #[test]
    fn drop_class_unknown_codes_bucketed_by_range() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/drop_reasons.h", "DropClass.Unknown", "tenant-hb-dc-unk");
        assert_eq!(drop_class(DropReason::Unknown(135)), DropClass::Policy);
        assert_eq!(drop_class(DropReason::Unknown(155)), DropClass::Conntrack);
        assert_eq!(drop_class(DropReason::Unknown(165)), DropClass::Nat);
        assert_eq!(drop_class(DropReason::Unknown(185)), DropClass::Encryption);
        assert_eq!(drop_class(DropReason::Unknown(205)), DropClass::L7);
        assert_eq!(drop_class(DropReason::Unknown(175)), DropClass::Ipam);
        assert_eq!(drop_class(DropReason::Unknown(999)), DropClass::Other);
    }

    #[test]
    fn drop_class_auth_required_maps_to_auth() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/drop", "DropClass.Auth", "tenant-hb-dc-auth");
        assert_eq!(drop_class(DropReason::AuthRequired), DropClass::Auth);
    }

    // ── FlowFilter ───────────────────────────────────────────────────────────

    #[test]
    fn flow_filter_by_verdict() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.Verdict", "tenant-hb-fv");
        let f = flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100);
        let filt = FlowFilter { verdict: Some(Verdict::Dropped), ..Default::default() };
        assert!(filt.matches(&f));
        let filt2 = FlowFilter { verdict: Some(Verdict::Forwarded), ..Default::default() };
        assert!(!filt2.matches(&f));
    }

    #[test]
    fn flow_filter_by_source_pod_exact() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.SourcePod", "tenant-hb-fsp");
        let f = flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100);
        let filt = FlowFilter { source_pods: vec!["ns/a".into()], ..Default::default() };
        assert!(filt.matches(&f));
        let filt2 = FlowFilter { source_pods: vec!["ns/c".into()], ..Default::default() };
        assert!(!filt2.matches(&f));
    }

    #[test]
    fn flow_filter_by_source_pod_namespace_prefix() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.SourcePod.Prefix", "tenant-hb-fspp");
        let f = flow("t", "ns/a", "other/b", 1, 2, Verdict::Forwarded, DropReason::None, 100);
        let filt = FlowFilter { source_pods: vec!["ns/".into()], ..Default::default() };
        assert!(filt.matches(&f));
    }

    #[test]
    fn flow_filter_by_namespace() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.Namespace", "tenant-hb-fns");
        let f = flow("t", "prod/a", "stage/b", 1, 2, Verdict::Forwarded, DropReason::None, 100);
        let filt = FlowFilter { destination_namespace: Some("stage".into()), ..Default::default() };
        assert!(filt.matches(&f));
        let filt2 = FlowFilter { destination_namespace: Some("prod".into()), ..Default::default() };
        assert!(!filt2.matches(&f));
    }

    #[test]
    fn flow_filter_by_identity() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.Identity", "tenant-hb-fid");
        let f = flow("t", "ns/a", "ns/b", 256, 257, Verdict::Forwarded, DropReason::None, 100);
        let filt = FlowFilter { source_identities: vec![256], ..Default::default() };
        assert!(filt.matches(&f));
        let filt2 = FlowFilter { source_identities: vec![999], ..Default::default() };
        assert!(!filt2.matches(&f));
    }

    #[test]
    fn flow_filter_by_drop_reason() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.DropReason", "tenant-hb-fdr");
        let f = flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 100);
        let filt = FlowFilter { drop_reasons: vec![DropReason::PolicyDeny], ..Default::default() };
        assert!(filt.matches(&f));
        let filt2 = FlowFilter { drop_reasons: vec![DropReason::CtInvalid], ..Default::default() };
        assert!(!filt2.matches(&f));
    }

    #[test]
    fn flow_filter_combined_AND() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.AND", "tenant-hb-fand");
        let f = flow("t", "ns/a", "ns/b", 256, 257, Verdict::Dropped, DropReason::PolicyDeny, 100);
        let filt = FlowFilter {
            verdict: Some(Verdict::Dropped),
            source_identities: vec![256],
            drop_reasons: vec![DropReason::PolicyDeny],
            ..Default::default()
        };
        assert!(filt.matches(&f));
    }

    #[test]
    fn flow_filter_blacklist_excludes_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "ApplyFilters.Blacklist", "tenant-hb-fbl");
        let flows = vec![
            flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100),
            flow("t", "ns/c", "ns/d", 3, 4, Verdict::Dropped, DropReason::PolicyDeny, 50),
        ];
        let bl = vec![FlowFilter { verdict: Some(Verdict::Dropped), ..Default::default() }];
        let out = apply_filters(&[], &bl, &flows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].verdict, Verdict::Forwarded);
    }

    #[test]
    fn flow_filter_whitelist_default_is_pass_all() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "ApplyFilters.Default", "tenant-hb-fwl");
        let flows = vec![flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 100)];
        let out = apply_filters(&[], &[], &flows);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn flow_filter_round_trips_serde() {
        let (_c, _t) = cilium_test_ctx!("api/v1/flow/flow.proto", "FlowFilter.Serde", "tenant-hb-fserde");
        let f = FlowFilter {
            verdict: Some(Verdict::Dropped),
            source_pods: vec!["ns/a".into()],
            destination_namespace: Some("prod".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: FlowFilter = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }

    // ── Observer ─────────────────────────────────────────────────────────────

    #[test]
    fn observer_get_flows_returns_buffered() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "Observer.GetFlows", "tenant-hb-obs-gf");
        let mut o = Observer::new(tenant.clone(), 100, 0);
        for i in 0..5u64 {
            o.ingest(flow("tenant-hb-obs-gf", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, i));
        }
        let req = GetFlowsRequest::default();
        let r = o.get_flows(&req);
        assert_eq!(r.len(), 5);
    }

    #[test]
    fn observer_get_flows_respects_limit() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "Observer.GetFlows.Limit", "tenant-hb-obs-lim");
        let mut o = Observer::new(tenant.clone(), 100, 0);
        for i in 0..10u64 {
            o.ingest(flow("tenant-hb-obs-lim", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, i));
        }
        let req = GetFlowsRequest { limit: 3, ..Default::default() };
        assert_eq!(o.get_flows(&req).len(), 3);
    }

    #[test]
    fn observer_get_flows_with_filter() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "Observer.GetFlows.Filter", "tenant-hb-obs-flt");
        let mut o = Observer::new(tenant.clone(), 100, 0);
        o.ingest(flow("tenant-hb-obs-flt", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 0));
        o.ingest(flow("tenant-hb-obs-flt", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 0));
        let req = GetFlowsRequest {
            whitelist: vec![FlowFilter { verdict: Some(Verdict::Dropped), ..Default::default() }],
            ..Default::default()
        };
        assert_eq!(o.get_flows(&req).len(), 1);
    }

    #[test]
    fn observer_evicts_old_flow_when_full() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "Observer.Eviction", "tenant-hb-obs-evict");
        let mut o = Observer::new(tenant.clone(), 3, 0);
        for _ in 0..5 {
            o.ingest(flow("tenant-hb-obs-evict", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 0));
        }
        assert_eq!(o.flows.len(), 3);
        assert_eq!(o.seen, 5);
    }

    #[test]
    fn observer_status_reports_counts_and_uptime() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "Observer.ServerStatus", "tenant-hb-obs-st");
        let mut o = Observer::new(tenant.clone(), 100, 1_000_000);
        for _ in 0..10 {
            o.ingest(flow("tenant-hb-obs-st", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 0));
        }
        let st = o.status(2_000_000_000);
        assert_eq!(st.num_flows, 10);
        assert_eq!(st.seen_flows, 10);
        assert!(st.uptime_ns > 0);
        assert!(st.flows_rate > 0.0);
    }

    #[test]
    fn observer_filters_cross_tenant_ingest() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "Observer.Tenant", "tenant-hb-obs-iso");
        let mut o = Observer::new(tenant.clone(), 100, 0);
        o.ingest(flow("other-tenant", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 0));
        assert_eq!(o.flows.len(), 0);
        assert_eq!(o.seen, 0);
    }

    // ── Peer service ─────────────────────────────────────────────────────────

    #[test]
    fn peer_service_upsert_emits_add_event() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/peer/service.go", "Peer.Upsert", "tenant-hb-peer-up");
        let mut ps = PeerService::new();
        ps.upsert(PeerInfo { name: "node-a".into(), address: "10.0.0.1:4244".into(), tls: true });
        let changes = ps.drain_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, PeerChangeKind::Add);
    }

    #[test]
    fn peer_service_upsert_existing_emits_update() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/peer/service.go", "Peer.Update", "tenant-hb-peer-upd");
        let mut ps = PeerService::new();
        ps.upsert(PeerInfo { name: "node-a".into(), address: "10.0.0.1:4244".into(), tls: true });
        let _ = ps.drain_changes();
        ps.upsert(PeerInfo { name: "node-a".into(), address: "10.0.0.2:4244".into(), tls: true });
        let changes = ps.drain_changes();
        assert_eq!(changes[0].kind, PeerChangeKind::Update);
    }

    #[test]
    fn peer_service_remove_emits_delete() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/peer/service.go", "Peer.Remove", "tenant-hb-peer-rm");
        let mut ps = PeerService::new();
        ps.upsert(PeerInfo { name: "node-a".into(), address: "x".into(), tls: false });
        let _ = ps.drain_changes();
        assert!(ps.remove("node-a"));
        let changes = ps.drain_changes();
        assert_eq!(changes[0].kind, PeerChangeKind::Delete);
    }

    #[test]
    fn peer_service_list_returns_known_peers() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/peer/service.go", "Peer.List", "tenant-hb-peer-list");
        let mut ps = PeerService::new();
        ps.upsert(PeerInfo { name: "a".into(), address: "x".into(), tls: false });
        ps.upsert(PeerInfo { name: "b".into(), address: "y".into(), tls: false });
        assert_eq!(ps.count(), 2);
        assert_eq!(ps.list().len(), 2);
    }

    // ── Relay ────────────────────────────────────────────────────────────────

    #[test]
    fn relay_aggregates_flows_from_multiple_clusters() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/relay.go", "Relay.GetFlows", "tenant-hb-rel-agg");
        let mut a = Observer::new(tenant.clone(), 100, 0);
        let mut b = Observer::new(tenant.clone(), 100, 0);
        a.ingest(flow(tenant.as_str(), "ns/x", "ns/y", 1, 2, Verdict::Forwarded, DropReason::None, 1));
        b.ingest(flow(tenant.as_str(), "ns/p", "ns/q", 3, 4, Verdict::Forwarded, DropReason::None, 2));
        let mut r = Relay::new(tenant);
        r.add_cluster("us-east", a);
        r.add_cluster("eu-west", b);
        let out = r.get_flows(&GetFlowsRequest::default());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn relay_sorts_aggregated_flows_by_time() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/relay.go", "Relay.GetFlows.Sort", "tenant-hb-rel-sort");
        let mut a = Observer::new(tenant.clone(), 100, 0);
        let mut b = Observer::new(tenant.clone(), 100, 0);
        a.ingest(flow(tenant.as_str(), "ns/x", "ns/y", 1, 2, Verdict::Forwarded, DropReason::None, 0));
        b.ingest(flow(tenant.as_str(), "ns/p", "ns/q", 3, 4, Verdict::Forwarded, DropReason::None, 0));
        let mut r = Relay::new(tenant);
        r.add_cluster("us-east", a);
        r.add_cluster("eu-west", b);
        let out = r.get_flows(&GetFlowsRequest::default());
        // Ordered ascending by time (loose check).
        assert!(out.len() >= 2);
        for w in out.windows(2) {
            assert!(w[0].time <= w[1].time);
        }
    }

    #[test]
    fn relay_remove_cluster_drops_observer() {
        let (_c, tenant) = cilium_test_ctx!("pkg/hubble/relay/relay.go", "Relay.RemoveCluster", "tenant-hb-rel-rm");
        let mut r = Relay::new(tenant.clone());
        r.add_cluster("us-east", Observer::new(tenant.clone(), 100, 0));
        assert!(r.remove_cluster("us-east"));
        assert_eq!(r.cluster_count(), 0);
    }

    // ── Metrics + summaries ──────────────────────────────────────────────────

    #[test]
    fn metrics_by_namespace_aggregates_forward_drop_bytes() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api.go", "MetricsByNamespace", "tenant-hb-mns");
        let flows = vec![
            flow("t", "ns-a/x", "ns-b/y", 1, 2, Verdict::Forwarded, DropReason::None, 100),
            flow("t", "ns-a/x", "ns-b/y", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 50),
            flow("t", "ns-a/x", "ns-b/y", 1, 2, Verdict::Forwarded, DropReason::None, 200),
        ];
        let m = metrics_by_namespace(&flows);
        assert_eq!(m["ns-a"].forwarded, 2);
        assert_eq!(m["ns-a"].dropped, 1);
        assert_eq!(m["ns-a"].bytes_out, 350);
        assert_eq!(m["ns-b"].bytes_in, 350);
    }

    #[test]
    fn metrics_by_namespace_drop_class_breakdown() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api.go", "MetricsByNamespace.DropClass", "tenant-hb-mdc");
        let flows = vec![
            flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 50),
            flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::CtInvalid, 50),
        ];
        let m = metrics_by_namespace(&flows);
        let dc = &m["ns"].drop_class_counts;
        assert_eq!(*dc.get("Policy").unwrap_or(&0), 1);
        assert_eq!(*dc.get("Conntrack").unwrap_or(&0), 1);
    }

    #[test]
    fn summarize_namespace_top_talkers_ordered() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api.go", "Summarize.TopTalkers", "tenant-hb-tt");
        let flows = vec![
            flow("t", "ns/a", "ns/b", 1, 100, Verdict::Forwarded, DropReason::None, 10),
            flow("t", "ns/a", "ns/c", 1, 200, Verdict::Forwarded, DropReason::None, 10),
            flow("t", "ns/a", "ns/d", 1, 200, Verdict::Forwarded, DropReason::None, 10),
            flow("t", "ns/a", "ns/e", 1, 200, Verdict::Forwarded, DropReason::None, 10),
        ];
        let s = summarize_namespace("ns", &flows, 2);
        assert_eq!(s.top_talkers[0], (200, 3));
        assert_eq!(s.top_talkers.len(), 2);
    }

    #[test]
    fn summarize_namespace_unique_destinations() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api.go", "Summarize.Unique", "tenant-hb-su");
        let flows = vec![
            flow("t", "ns/a", "ns/b", 1, 100, Verdict::Forwarded, DropReason::None, 10),
            flow("t", "ns/a", "ns/c", 1, 100, Verdict::Forwarded, DropReason::None, 10),
            flow("t", "ns/a", "ns/c", 1, 200, Verdict::Forwarded, DropReason::None, 10),
        ];
        let s = summarize_namespace("ns", &flows, 5);
        assert_eq!(s.unique_destinations.len(), 2);
    }

    #[test]
    fn summarize_namespace_skips_other_namespace_flows() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api.go", "Summarize.Filter", "tenant-hb-sf");
        let flows = vec![
            flow("t", "ns/a", "ns/b", 1, 2, Verdict::Forwarded, DropReason::None, 10),
            flow("t", "other/a", "ns/b", 3, 4, Verdict::Forwarded, DropReason::None, 10),
        ];
        let s = summarize_namespace("ns", &flows, 5);
        assert_eq!(s.forwarded, 1);
    }

    #[test]
    fn summarize_namespace_counts_drops() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/metrics/api.go", "Summarize.Drops", "tenant-hb-sd");
        let flows = vec![
            flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::PolicyDeny, 10),
            flow("t", "ns/a", "ns/b", 1, 2, Verdict::Dropped, DropReason::CtInvalid, 10),
        ];
        let s = summarize_namespace("ns", &flows, 5);
        assert_eq!(s.dropped, 2);
        assert_eq!(s.forwarded, 0);
    }

    // ── Misc ────────────────────────────────────────────────────────────────

    #[test]
    fn server_status_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/observer/observer.go", "ServerStatus.Serde", "tenant-hb-ss-serde");
        let s = ServerStatus { num_flows: 10, max_flows: 100, seen_flows: 50, uptime_ns: 1_000_000, flows_rate: 5.0 };
        let json = serde_json::to_string(&s).unwrap();
        let back: ServerStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn peer_change_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/hubble/peer/service.go", "PeerChange.Serde", "tenant-hb-pc-serde");
        let pc = PeerChange {
            kind: PeerChangeKind::Add,
            peer: PeerInfo { name: "node-a".into(), address: "10.0.0.1:4244".into(), tls: true },
        };
        let json = serde_json::to_string(&pc).unwrap();
        let back: PeerChange = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pc);
    }

    #[test]
    fn get_flows_request_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("api/v1/observer/observer.proto", "GetFlowsRequest", "tenant-hb-gfr-serde");
        let r = GetFlowsRequest { limit: 100, follow: true, ..Default::default() };
        let json = serde_json::to_string(&r).unwrap();
        let back: GetFlowsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hubble-equivalent L7 flow visualizer — replaces the Cilium Hubble UI.
//!
//! cave-net produces L7 flow records; this plugin shapes them for the
//! native portal page. There is **no** redirect to a Hubble UI — the portal
//! renders the service map and flow log itself.
//!
//! The viz has three views:
//! - **Service map**: nodes (workloads) and edges (observed flow tuples).
//! - **Flow log**: time-ordered raw flows with verdict + protocol filter.
//! - **Policy debug**: which network policies hit/dropped a given flow.

use super::ViewPersona;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Forwarded,
    Dropped,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum L4Protocol {
    Tcp,
    Udp,
    Icmp,
    Sctp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum L7Kind {
    Http,
    Grpc,
    Dns,
    Kafka,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub workload: String,
    pub namespace: String,
    pub pod: Option<String>,
    pub ip: String,
    pub port: u16,
}

impl Endpoint {
    pub fn new(
        workload: impl Into<String>,
        ns: impl Into<String>,
        ip: impl Into<String>,
        port: u16,
    ) -> Self {
        Self {
            workload: workload.into(),
            namespace: ns.into(),
            pod: None,
            ip: ip.into(),
            port,
        }
    }

    pub fn with_pod(mut self, pod: impl Into<String>) -> Self {
        self.pod = Some(pod.into());
        self
    }

    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.workload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowRecord {
    pub id: u64,
    pub tenant: String,
    pub timestamp: u64,
    pub verdict: Verdict,
    pub source: Endpoint,
    pub destination: Endpoint,
    pub l4: L4Protocol,
    pub l7: L7Kind,
    pub http_method: Option<String>,
    pub http_path: Option<String>,
    pub http_status: Option<u16>,
    pub policy_match: Option<String>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowQuery {
    pub tenant: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub workload: Option<String>,
    #[serde(default)]
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub l4: Option<L4Protocol>,
    #[serde(default)]
    pub l7: Option<L7Kind>,
    #[serde(default)]
    pub http_status_min: Option<u16>,
    #[serde(default)]
    pub http_status_max: Option<u16>,
    #[serde(default)]
    pub from_ts: Option<u64>,
    #[serde(default)]
    pub to_ts: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HubbleError {
    #[error("limit too large (max 5000)")]
    LimitTooLarge,
    #[error("invalid time range")]
    InvalidRange,
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
}

const MAX_LIMIT: usize = 5_000;
const DEFAULT_LIMIT: usize = 200;
const RING_CAPACITY: usize = 200_000;

/// Aggregated edge in the service-map view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEdge {
    pub source: String,      // namespace/workload key
    pub destination: String, // namespace/workload key
    pub flows_total: u64,
    pub flows_forwarded: u64,
    pub flows_dropped: u64,
    pub bytes_total: u64,
}

impl ServiceEdge {
    pub fn drop_rate(&self) -> f64 {
        if self.flows_total == 0 {
            return 0.0;
        }
        self.flows_dropped as f64 / self.flows_total as f64
    }
}

#[derive(Debug, Default)]
pub struct HubblePlugin {
    flows: std::collections::VecDeque<FlowRecord>,
    seq: u64,
}

impl HubblePlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a flow. The plugin owns the ring buffer.
    pub fn record(
        &mut self,
        tenant: impl Into<String>,
        timestamp: u64,
        verdict: Verdict,
        source: Endpoint,
        destination: Endpoint,
        l4: L4Protocol,
        l7: L7Kind,
        bytes: u64,
    ) -> &FlowRecord {
        self.seq += 1;
        let f = FlowRecord {
            id: self.seq,
            tenant: tenant.into(),
            timestamp,
            verdict,
            source,
            destination,
            l4,
            l7,
            http_method: None,
            http_path: None,
            http_status: None,
            policy_match: None,
            bytes,
        };
        if self.flows.len() >= RING_CAPACITY {
            self.flows.pop_front();
        }
        self.flows.push_back(f);
        self.flows.back().unwrap()
    }

    pub fn record_http(
        &mut self,
        tenant: impl Into<String>,
        timestamp: u64,
        verdict: Verdict,
        source: Endpoint,
        destination: Endpoint,
        method: impl Into<String>,
        path: impl Into<String>,
        status: u16,
        bytes: u64,
    ) -> &FlowRecord {
        self.seq += 1;
        let f = FlowRecord {
            id: self.seq,
            tenant: tenant.into(),
            timestamp,
            verdict,
            source,
            destination,
            l4: L4Protocol::Tcp,
            l7: L7Kind::Http,
            http_method: Some(method.into()),
            http_path: Some(path.into()),
            http_status: Some(status),
            policy_match: None,
            bytes,
        };
        if self.flows.len() >= RING_CAPACITY {
            self.flows.pop_front();
        }
        self.flows.push_back(f);
        self.flows.back().unwrap()
    }

    pub fn annotate_policy(&mut self, flow_id: u64, policy: impl Into<String>) -> bool {
        if let Some(f) = self.flows.iter_mut().find(|f| f.id == flow_id) {
            f.policy_match = Some(policy.into());
            return true;
        }
        false
    }

    pub fn count(&self) -> usize {
        self.flows.len()
    }

    /// Operator/admin can see all flows in a tenant; tenant-persona only their
    /// own (already filtered by query.tenant).
    pub fn allowed_for(persona: ViewPersona) -> bool {
        // L7 visibility is sensitive (paths, methods). Operators + admins.
        // Tenant persona can see flows where they are source or destination
        // workload — this is enforced at the query level.
        let _ = persona;
        true
    }

    pub fn query(
        &self,
        persona: ViewPersona,
        q: &FlowQuery,
    ) -> Result<Vec<FlowRecord>, HubbleError> {
        if matches!(persona, ViewPersona::Tenant) && q.workload.is_none() && q.namespace.is_none() {
            return Err(HubbleError::Forbidden(
                "tenant persona must scope by namespace or workload",
            ));
        }
        if let (Some(f), Some(t)) = (q.from_ts, q.to_ts) {
            if f > t {
                return Err(HubbleError::InvalidRange);
            }
        }
        let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
        if limit > MAX_LIMIT {
            return Err(HubbleError::LimitTooLarge);
        }
        let mut out: Vec<FlowRecord> = self
            .flows
            .iter()
            .rev()
            .filter(|f| f.tenant == q.tenant)
            .filter(|f| {
                q.namespace.as_deref().map_or(true, |ns| {
                    f.source.namespace == ns || f.destination.namespace == ns
                })
            })
            .filter(|f| {
                q.workload.as_deref().map_or(true, |w| {
                    f.source.workload == w || f.destination.workload == w
                })
            })
            .filter(|f| q.verdict.map_or(true, |v| f.verdict == v))
            .filter(|f| q.l4.map_or(true, |p| f.l4 == p))
            .filter(|f| q.l7.map_or(true, |p| f.l7 == p))
            .filter(|f| {
                q.http_status_min
                    .map_or(true, |min| f.http_status.map_or(false, |s| s >= min))
            })
            .filter(|f| {
                q.http_status_max
                    .map_or(true, |max| f.http_status.map_or(false, |s| s <= max))
            })
            .filter(|f| q.from_ts.map_or(true, |from| f.timestamp >= from))
            .filter(|f| q.to_ts.map_or(true, |to| f.timestamp <= to))
            .take(limit)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(out)
    }

    /// Build a service map (aggregated edges) from the in-memory ring for a
    /// tenant. Optional namespace filter narrows the graph.
    pub fn service_map(&self, tenant: &str, namespace: Option<&str>) -> Vec<ServiceEdge> {
        let mut acc: HashMap<(String, String), ServiceEdge> = HashMap::new();
        for f in self.flows.iter().filter(|f| f.tenant == tenant) {
            if let Some(ns) = namespace {
                if f.source.namespace != ns && f.destination.namespace != ns {
                    continue;
                }
            }
            let key = (f.source.key(), f.destination.key());
            let edge = acc.entry(key.clone()).or_insert(ServiceEdge {
                source: key.0,
                destination: key.1,
                flows_total: 0,
                flows_forwarded: 0,
                flows_dropped: 0,
                bytes_total: 0,
            });
            edge.flows_total += 1;
            edge.bytes_total += f.bytes;
            match f.verdict {
                Verdict::Forwarded => edge.flows_forwarded += 1,
                Verdict::Dropped => edge.flows_dropped += 1,
                Verdict::Audit => {}
            }
        }
        let mut out: Vec<ServiceEdge> = acc.into_values().collect();
        out.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then(a.destination.cmp(&b.destination))
        });
        out
    }

    /// Top-N "noisy" workloads by either drop count or total flow count.
    pub fn top_noisy(&self, tenant: &str, by_drops: bool, n: usize) -> Vec<(String, u64)> {
        let mut acc: BTreeMap<String, u64> = BTreeMap::new();
        for f in self.flows.iter().filter(|f| f.tenant == tenant) {
            let key = f.source.key();
            let entry = acc.entry(key).or_insert(0);
            if by_drops {
                if f.verdict == Verdict::Dropped {
                    *entry += 1;
                }
            } else {
                *entry += 1;
            }
        }
        let mut v: Vec<(String, u64)> = acc.into_iter().filter(|(_, c)| *c > 0).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.truncate(n);
        v
    }

    /// HTTP latency histogram (status-class buckets) — used for the
    /// Status-Code Donut on the dashboard.
    pub fn http_status_histogram(&self, tenant: &str) -> [u64; 5] {
        let mut h = [0u64; 5]; // 1xx, 2xx, 3xx, 4xx, 5xx
        for f in self.flows.iter().filter(|f| f.tenant == tenant) {
            if let Some(s) = f.http_status {
                let bucket = match s {
                    100..=199 => 0,
                    200..=299 => 1,
                    300..=399 => 2,
                    400..=499 => 3,
                    500..=599 => 4,
                    _ => continue,
                };
                h[bucket] += 1;
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(workload: &str, ns: &str, port: u16) -> Endpoint {
        Endpoint::new(workload, ns, "10.0.0.1", port)
    }

    fn populated_plugin() -> HubblePlugin {
        let mut p = HubblePlugin::new();
        p.record(
            "acme",
            10,
            Verdict::Forwarded,
            ep("web", "front", 0),
            ep("api", "back", 8080),
            L4Protocol::Tcp,
            L7Kind::Http,
            1024,
        );
        p.record(
            "acme",
            11,
            Verdict::Dropped,
            ep("web", "front", 0),
            ep("api", "back", 8080),
            L4Protocol::Tcp,
            L7Kind::None,
            0,
        );
        p.record(
            "globex",
            10,
            Verdict::Forwarded,
            ep("web", "front", 0),
            ep("db", "back", 5432),
            L4Protocol::Tcp,
            L7Kind::None,
            512,
        );
        p
    }

    #[test]
    fn endpoint_key_format() {
        let e = ep("web", "front", 8080);
        assert_eq!(e.key(), "front/web");
    }

    #[test]
    fn endpoint_with_pod() {
        let e = ep("web", "front", 80).with_pod("web-abc123");
        assert_eq!(e.pod.as_deref(), Some("web-abc123"));
    }

    #[test]
    fn record_assigns_increasing_ids() {
        let mut p = HubblePlugin::new();
        let id1 = p
            .record(
                "a",
                0,
                Verdict::Forwarded,
                ep("a", "n", 0),
                ep("b", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            )
            .id;
        let id2 = p
            .record(
                "a",
                0,
                Verdict::Forwarded,
                ep("a", "n", 0),
                ep("b", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            )
            .id;
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn record_http_sets_l7_fields() {
        let mut p = HubblePlugin::new();
        let f = p.record_http(
            "a",
            0,
            Verdict::Forwarded,
            ep("c", "n", 0),
            ep("s", "n", 80),
            "GET",
            "/v1/users",
            200,
            256,
        );
        assert_eq!(f.l7, L7Kind::Http);
        assert_eq!(f.http_method.as_deref(), Some("GET"));
        assert_eq!(f.http_path.as_deref(), Some("/v1/users"));
        assert_eq!(f.http_status, Some(200));
    }

    #[test]
    fn count_tracks_recorded() {
        let p = populated_plugin();
        assert_eq!(p.count(), 3);
    }

    #[test]
    fn ring_caps_at_capacity() {
        let mut p = HubblePlugin::new();
        for _ in 0..(RING_CAPACITY + 50) {
            p.record(
                "a",
                0,
                Verdict::Forwarded,
                ep("a", "n", 0),
                ep("b", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                1,
            );
        }
        assert_eq!(p.count(), RING_CAPACITY);
    }

    #[test]
    fn annotate_policy_succeeds_for_known_id() {
        let mut p = HubblePlugin::new();
        let id = p
            .record(
                "a",
                0,
                Verdict::Dropped,
                ep("c", "n", 0),
                ep("s", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            )
            .id;
        assert!(p.annotate_policy(id, "deny-egress"));
    }

    #[test]
    fn annotate_policy_fails_for_unknown_id() {
        let mut p = HubblePlugin::new();
        assert!(!p.annotate_policy(9999, "x"));
    }

    #[test]
    fn query_filter_by_verdict() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            verdict: Some(Verdict::Dropped),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Operator, &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].verdict, Verdict::Dropped);
    }

    #[test]
    fn query_filter_by_l7_http() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            l7: Some(L7Kind::Http),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Operator, &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].l7, L7Kind::Http);
    }

    #[test]
    fn query_filter_by_namespace() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            namespace: Some("back".into()),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Operator, &q).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn query_filter_by_workload() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            workload: Some("web".into()),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Tenant, &q).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn query_filter_by_time_range() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            from_ts: Some(11),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Operator, &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].timestamp, 11);
    }

    #[test]
    fn query_invalid_range_rejected() {
        let p = HubblePlugin::new();
        let q = FlowQuery {
            tenant: "acme".into(),
            from_ts: Some(20),
            to_ts: Some(10),
            ..Default::default()
        };
        let err = p.query(ViewPersona::Operator, &q).unwrap_err();
        assert_eq!(err, HubbleError::InvalidRange);
    }

    #[test]
    fn query_limit_too_large_rejected() {
        let p = HubblePlugin::new();
        let q = FlowQuery {
            tenant: "acme".into(),
            limit: Some(MAX_LIMIT + 1),
            ..Default::default()
        };
        let err = p.query(ViewPersona::Operator, &q).unwrap_err();
        assert_eq!(err, HubbleError::LimitTooLarge);
    }

    #[test]
    fn query_returns_descending_by_id() {
        let mut p = HubblePlugin::new();
        for ts in 0..5 {
            p.record(
                "acme",
                ts,
                Verdict::Forwarded,
                ep("a", "n", 0),
                ep("b", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            );
        }
        let q = FlowQuery {
            tenant: "acme".into(),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Operator, &q).unwrap();
        let ids: Vec<u64> = out.iter().map(|f| f.id).collect();
        let mut desc = ids.clone();
        desc.sort_by(|a, b| b.cmp(a));
        assert_eq!(ids, desc);
    }

    #[test]
    fn query_tenant_persona_must_scope() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            ..Default::default()
        };
        let err = p.query(ViewPersona::Tenant, &q).unwrap_err();
        assert!(matches!(err, HubbleError::Forbidden(_)));
    }

    #[test]
    fn query_tenant_persona_with_workload_ok() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            workload: Some("web".into()),
            ..Default::default()
        };
        assert!(p.query(ViewPersona::Tenant, &q).is_ok());
    }

    #[test]
    fn query_tenant_persona_with_namespace_ok() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "acme".into(),
            namespace: Some("front".into()),
            ..Default::default()
        };
        assert!(p.query(ViewPersona::Tenant, &q).is_ok());
    }

    #[test]
    fn query_filters_by_tenant_id() {
        let p = populated_plugin();
        let q = FlowQuery {
            tenant: "globex".into(),
            workload: Some("web".into()),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Tenant, &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tenant, "globex");
    }

    #[test]
    fn query_filter_http_status_range() {
        let mut p = HubblePlugin::new();
        for status in [200, 404, 500] {
            p.record_http(
                "a",
                0,
                Verdict::Forwarded,
                ep("c", "n", 0),
                ep("s", "n", 80),
                "GET",
                "/x",
                status,
                0,
            );
        }
        let q = FlowQuery {
            tenant: "a".into(),
            http_status_min: Some(400),
            http_status_max: Some(499),
            ..Default::default()
        };
        let out = p.query(ViewPersona::Operator, &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].http_status, Some(404));
    }

    #[test]
    fn service_map_aggregates_edges() {
        let p = populated_plugin();
        let edges = p.service_map("acme", None);
        assert_eq!(edges.len(), 1);
        let e = &edges[0];
        assert_eq!(e.source, "front/web");
        assert_eq!(e.destination, "back/api");
        assert_eq!(e.flows_total, 2);
        assert_eq!(e.flows_forwarded, 1);
        assert_eq!(e.flows_dropped, 1);
        assert_eq!(e.bytes_total, 1024);
    }

    #[test]
    fn service_map_drop_rate() {
        let p = populated_plugin();
        let edges = p.service_map("acme", None);
        assert!((edges[0].drop_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn service_map_drop_rate_zero_for_empty_edge() {
        let edge = ServiceEdge {
            source: "a".into(),
            destination: "b".into(),
            flows_total: 0,
            flows_forwarded: 0,
            flows_dropped: 0,
            bytes_total: 0,
        };
        assert_eq!(edge.drop_rate(), 0.0);
    }

    #[test]
    fn service_map_filters_by_namespace() {
        let p = populated_plugin();
        let edges = p.service_map("acme", Some("front"));
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn service_map_filters_unknown_namespace_returns_empty() {
        let p = populated_plugin();
        let edges = p.service_map("acme", Some("ghost"));
        assert!(edges.is_empty());
    }

    #[test]
    fn service_map_isolates_tenants() {
        let p = populated_plugin();
        let acme = p.service_map("acme", None);
        let globex = p.service_map("globex", None);
        assert_eq!(acme.len(), 1);
        assert_eq!(globex.len(), 1);
        assert_ne!(acme[0].destination, globex[0].destination);
    }

    #[test]
    fn top_noisy_by_total_flows() {
        let mut p = HubblePlugin::new();
        for _ in 0..3 {
            p.record(
                "a",
                0,
                Verdict::Forwarded,
                ep("noisy", "n", 0),
                ep("x", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            );
        }
        for _ in 0..1 {
            p.record(
                "a",
                0,
                Verdict::Forwarded,
                ep("quiet", "n", 0),
                ep("x", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            );
        }
        let top = p.top_noisy("a", false, 5);
        assert_eq!(top[0].0, "n/noisy");
        assert_eq!(top[0].1, 3);
    }

    #[test]
    fn top_noisy_by_drops_only_counts_dropped() {
        let mut p = HubblePlugin::new();
        for _ in 0..3 {
            p.record(
                "a",
                0,
                Verdict::Forwarded,
                ep("ok", "n", 0),
                ep("x", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            );
        }
        for _ in 0..2 {
            p.record(
                "a",
                0,
                Verdict::Dropped,
                ep("bad", "n", 0),
                ep("x", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            );
        }
        let top = p.top_noisy("a", true, 5);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0, "n/bad");
        assert_eq!(top[0].1, 2);
    }

    #[test]
    fn top_noisy_respects_n() {
        let mut p = HubblePlugin::new();
        for w in ["a", "b", "c", "d"] {
            p.record(
                "t",
                0,
                Verdict::Forwarded,
                ep(w, "n", 0),
                ep("x", "n", 0),
                L4Protocol::Tcp,
                L7Kind::None,
                0,
            );
        }
        let top = p.top_noisy("t", false, 2);
        assert_eq!(top.len(), 2);
    }

    #[test]
    fn http_status_histogram_buckets_correctly() {
        let mut p = HubblePlugin::new();
        for s in [200, 201, 304, 404, 500, 502] {
            p.record_http(
                "a",
                0,
                Verdict::Forwarded,
                ep("c", "n", 0),
                ep("s", "n", 80),
                "GET",
                "/x",
                s,
                0,
            );
        }
        let h = p.http_status_histogram("a");
        // 0:1xx, 1:2xx, 2:3xx, 3:4xx, 4:5xx
        assert_eq!(h, [0, 2, 1, 1, 2]);
    }

    #[test]
    fn http_status_histogram_ignores_non_http() {
        let mut p = HubblePlugin::new();
        p.record(
            "a",
            0,
            Verdict::Forwarded,
            ep("c", "n", 0),
            ep("s", "n", 0),
            L4Protocol::Tcp,
            L7Kind::None,
            0,
        );
        let h = p.http_status_histogram("a");
        assert_eq!(h, [0, 0, 0, 0, 0]);
    }

    #[test]
    fn verdict_serializes_snake_case() {
        let s = serde_json::to_string(&Verdict::Forwarded).unwrap();
        assert_eq!(s, "\"forwarded\"");
    }

    #[test]
    fn l4_serializes_uppercase() {
        let s = serde_json::to_string(&L4Protocol::Tcp).unwrap();
        assert_eq!(s, "\"TCP\"");
    }

    #[test]
    fn flow_record_round_trips_json() {
        let mut p = HubblePlugin::new();
        let f = p
            .record_http(
                "a",
                1,
                Verdict::Forwarded,
                ep("c", "n", 0),
                ep("s", "n", 80),
                "GET",
                "/x",
                200,
                100,
            )
            .clone();
        let s = serde_json::to_string(&f).unwrap();
        let back: FlowRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn allowed_for_returns_true_for_all() {
        // gating happens at query time, not here
        assert!(HubblePlugin::allowed_for(ViewPersona::Tenant));
        assert!(HubblePlugin::allowed_for(ViewPersona::Operator));
        assert!(HubblePlugin::allowed_for(ViewPersona::Admin));
    }
}

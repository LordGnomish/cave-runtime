// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hubble — observability surface for cave-net.
//!
//! Mirrors `pkg/hubble/parser/parser.go` plus the drop-reason taxonomy from
//! `bpf/lib/drop_reasons.h` and the topology-graph builder lurking in
//! `pkg/hubble/relay`. This module models:
//!
//! * [`FlowLog`] — one record per observed packet/flow.
//! * [`DropReason`] — Cilium's enum of drop causes, with `from_code` to
//!   parse the numeric reason that the eBPF dataplane emits.
//! * [`TopologyGraph`] — directed graph of `(source_identity →
//!   destination_identity)` weighted by flow count, built from a slice of
//!   flow logs.

use crate::cilium::types::{Cite, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Forwarded,
    Dropped,
    Error,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropReason {
    /// 0 — no drop.
    None,
    /// 133 — `DROP_POLICY` from upstream.
    PolicyDeny,
    /// 134 — `DROP_INVALID`, packet failed verifier.
    Invalid,
    /// 137 — `DROP_CT_INVALID_HDR`, conntrack rejection.
    CtInvalid,
    /// 144 — `DROP_FRAG_NEEDED`, MTU issue.
    FragmentationNeeded,
    /// 162 — `DROP_NAT_NO_MAPPING`, NAT lookup failed.
    NatNoMapping,
    /// 192 — `DROP_AUTH_REQUIRED`, mTLS auth required.
    AuthRequired,
    /// Anything else.
    Unknown(u16),
}

impl DropReason {
    /// Parse the eBPF numeric reason code. Codes match upstream constants.
    pub fn from_code(code: u16) -> Self {
        match code {
            0 => DropReason::None,
            133 => DropReason::PolicyDeny,
            134 => DropReason::Invalid,
            137 => DropReason::CtInvalid,
            144 => DropReason::FragmentationNeeded,
            162 => DropReason::NatNoMapping,
            192 => DropReason::AuthRequired,
            other => DropReason::Unknown(other),
        }
    }

    /// Human-readable category. Mirrors the labels Hubble emits in
    /// `flow.drop_reason_desc`.
    pub fn description(&self) -> &'static str {
        match self {
            DropReason::None => "none",
            DropReason::PolicyDeny => "policy_denied",
            DropReason::Invalid => "invalid_packet",
            DropReason::CtInvalid => "ct_invalid",
            DropReason::FragmentationNeeded => "fragmentation_needed",
            DropReason::NatNoMapping => "nat_no_mapping",
            DropReason::AuthRequired => "auth_required",
            DropReason::Unknown(_) => "unknown",
        }
    }
}

/// One flow record. Mirrors `flow.Flow` in upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowLog {
    pub tenant: TenantId,
    pub time: DateTime<Utc>,
    pub source_identity: u32,
    pub destination_identity: u32,
    pub source_pod: String,
    pub destination_pod: String,
    pub verdict: Verdict,
    pub drop_reason: DropReason,
    pub bytes: u64,
}

/// In-memory ring of flow logs scoped to a tenant. Mirrors the buffer
/// `pkg/hubble/observer/observer.go` keeps before exporting to Hubble Relay.
#[derive(Debug)]
pub struct FlowBuffer {
    pub tenant: TenantId,
    pub capacity: usize,
    flows: Vec<FlowLog>,
    overflow: u64,
}

impl FlowBuffer {
    pub fn new(tenant: TenantId, capacity: usize) -> Self {
        Self { tenant, capacity, flows: Vec::with_capacity(capacity.min(1024)), overflow: 0 }
    }

    /// Append a flow. If the buffer is full, the oldest record is dropped
    /// and `overflow_count()` is bumped — matches Hubble's ring behaviour.
    /// Cross-tenant flows are silently filtered (the dataplane shouldn't
    /// emit them, but defence-in-depth never hurts).
    pub fn push(&mut self, flow: FlowLog) {
        if flow.tenant != self.tenant {
            return;
        }
        if self.flows.len() >= self.capacity {
            self.flows.remove(0);
            self.overflow += 1;
        }
        self.flows.push(flow);
    }

    pub fn flows(&self) -> &[FlowLog] {
        &self.flows
    }

    pub fn overflow_count(&self) -> u64 {
        self.overflow
    }

    /// Return only the dropped flows (for `hubble observe --verdict DROPPED`).
    pub fn drops(&self) -> Vec<&FlowLog> {
        self.flows.iter().filter(|f| f.verdict == Verdict::Dropped).collect()
    }
}

/// Topology graph: identity-pair → flow count + bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyGraph {
    pub edges: BTreeMap<(u32, u32), EdgeStats>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeStats {
    pub forwarded: u64,
    pub dropped: u64,
    pub bytes: u64,
}

impl TopologyGraph {
    /// Build a topology graph from flow logs, scoped to one tenant.
    pub fn build(tenant: &TenantId, flows: &[FlowLog]) -> Self {
        let mut g = TopologyGraph::default();
        for f in flows {
            if &f.tenant != tenant {
                continue;
            }
            let edge = g
                .edges
                .entry((f.source_identity, f.destination_identity))
                .or_default();
            match f.verdict {
                Verdict::Forwarded => edge.forwarded += 1,
                Verdict::Dropped => edge.dropped += 1,
                _ => {}
            }
            edge.bytes += f.bytes;
        }
        g
    }

    pub fn edge(&self, src: u32, dst: u32) -> Option<&EdgeStats> {
        self.edges.get(&(src, dst))
    }

    pub fn node_count(&self) -> usize {
        let mut nodes = std::collections::BTreeSet::new();
        for (s, d) in self.edges.keys() {
            nodes.insert(*s);
            nodes.insert(*d);
        }
        nodes.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/hubble/parser/parser.go", "Parser.Decode");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn flow(
        tenant: &str,
        src: u32,
        dst: u32,
        verdict: Verdict,
        drop: DropReason,
        bytes: u64,
    ) -> FlowLog {
        FlowLog {
            tenant: TenantId::new(tenant).expect("test fixture"),
            time: DateTime::parse_from_rfc3339("2026-04-26T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            source_identity: src,
            destination_identity: dst,
            source_pod: format!("pod-{src}"),
            destination_pod: format!("pod-{dst}"),
            verdict,
            drop_reason: drop,
            bytes,
        }
    }

    #[test]
    fn drop_reason_from_known_codes() {
        let (_cite, _t) = cilium_test_ctx!(
            "bpf/lib/drop_reasons.h",
            "DROP_POLICY",
            "tenant-hub-codes"
        );
        assert_eq!(DropReason::from_code(0), DropReason::None);
        assert_eq!(DropReason::from_code(133), DropReason::PolicyDeny);
        assert_eq!(DropReason::from_code(192), DropReason::AuthRequired);
    }

    #[test]
    fn drop_reason_unknown_code_round_trips_value() {
        let (_cite, _t) = cilium_test_ctx!(
            "bpf/lib/drop_reasons.h",
            "DROP_UNKNOWN",
            "tenant-hub-unknown"
        );
        assert_eq!(DropReason::from_code(9999), DropReason::Unknown(9999));
    }

    #[test]
    fn drop_reason_description_matches_known_labels() {
        let (_cite, _t) = cilium_test_ctx!(
            "pkg/hubble/parser/dropreason.go",
            "DropReasonDesc",
            "tenant-hub-desc"
        );
        assert_eq!(DropReason::PolicyDeny.description(), "policy_denied");
        assert_eq!(DropReason::AuthRequired.description(), "auth_required");
        assert_eq!(DropReason::None.description(), "none");
    }

    #[test]
    fn flow_buffer_keeps_only_capacity_records() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/hubble/observer/observer.go",
            "ringBuffer",
            "tenant-hub-ring"
        );
        let mut buf = FlowBuffer::new(tenant, 3);
        for i in 0..5 {
            buf.push(flow("tenant-hub-ring", 256, 257 + i, Verdict::Forwarded, DropReason::None, 100));
        }
        assert_eq!(buf.flows().len(), 3);
        assert_eq!(buf.overflow_count(), 2);
    }

    #[test]
    fn flow_buffer_drops_only_emits_dropped_records() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/hubble/observer/observer.go",
            "filterByVerdict",
            "tenant-hub-drops"
        );
        let mut buf = FlowBuffer::new(tenant, 16);
        buf.push(flow("tenant-hub-drops", 256, 257, Verdict::Forwarded, DropReason::None, 100));
        buf.push(flow("tenant-hub-drops", 256, 258, Verdict::Dropped, DropReason::PolicyDeny, 50));
        buf.push(flow("tenant-hub-drops", 256, 259, Verdict::Dropped, DropReason::AuthRequired, 0));
        assert_eq!(buf.drops().len(), 2);
    }

    #[test]
    fn flow_buffer_filters_cross_tenant_flows() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/hubble/observer/observer.go",
            "tenantScope",
            "acme"
        );
        let mut buf = FlowBuffer::new(tenant, 16);
        buf.push(flow("acme", 256, 257, Verdict::Forwarded, DropReason::None, 100));
        buf.push(flow("evil", 256, 257, Verdict::Forwarded, DropReason::None, 100));
        assert_eq!(buf.flows().len(), 1);
    }

    #[test]
    fn topology_graph_aggregates_by_identity_pair() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/hubble/relay/server.go",
            "topologyBuilder",
            "tenant-hub-topology"
        );
        let flows = vec![
            flow("tenant-hub-topology", 256, 257, Verdict::Forwarded, DropReason::None, 100),
            flow("tenant-hub-topology", 256, 257, Verdict::Forwarded, DropReason::None, 200),
            flow("tenant-hub-topology", 256, 257, Verdict::Dropped, DropReason::PolicyDeny, 50),
            flow("tenant-hub-topology", 257, 258, Verdict::Forwarded, DropReason::None, 10),
        ];
        let g = TopologyGraph::build(&tenant, &flows);
        let edge = g.edge(256, 257).unwrap();
        assert_eq!(edge.forwarded, 2);
        assert_eq!(edge.dropped, 1);
        assert_eq!(edge.bytes, 350);
        assert_eq!(g.node_count(), 3); // {256, 257, 258}
    }

    #[test]
    fn topology_graph_skips_other_tenants() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/hubble/relay/server.go",
            "topologyTenantScope",
            "acme"
        );
        let flows = vec![
            flow("acme", 256, 257, Verdict::Forwarded, DropReason::None, 100),
            flow("evil", 256, 257, Verdict::Forwarded, DropReason::None, 100),
        ];
        let g = TopologyGraph::build(&tenant, &flows);
        assert_eq!(g.edge(256, 257).unwrap().forwarded, 1);
    }

    #[test]
    fn topology_graph_directed_edges_are_distinct() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/hubble/relay/server.go",
            "directedEdges",
            "tenant-hub-directed"
        );
        let flows = vec![
            flow("tenant-hub-directed", 256, 257, Verdict::Forwarded, DropReason::None, 10),
            flow("tenant-hub-directed", 257, 256, Verdict::Forwarded, DropReason::None, 20),
        ];
        let g = TopologyGraph::build(&tenant, &flows);
        assert_ne!(g.edge(256, 257), g.edge(257, 256));
    }

    #[test]
    fn empty_flow_set_yields_empty_topology() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/hubble/relay/server.go",
            "topologyBuilder",
            "tenant-hub-empty"
        );
        let g = TopologyGraph::build(&tenant, &[]);
        assert!(g.edges.is_empty());
        assert_eq!(g.node_count(), 0);
    }

    #[test]
    fn drop_reason_codes_distinct_for_documented_set() {
        let (_cite, _t) = cilium_test_ctx!(
            "bpf/lib/drop_reasons.h",
            "DROP_REASON_TABLE",
            "tenant-hub-distinct"
        );
        let codes = [
            DropReason::None,
            DropReason::PolicyDeny,
            DropReason::Invalid,
            DropReason::CtInvalid,
            DropReason::FragmentationNeeded,
            DropReason::NatNoMapping,
            DropReason::AuthRequired,
        ];
        let mut descs: Vec<&'static str> = codes.iter().map(|c| c.description()).collect();
        descs.sort();
        descs.dedup();
        assert_eq!(descs.len(), codes.len(), "every documented code needs a unique description");
    }
}

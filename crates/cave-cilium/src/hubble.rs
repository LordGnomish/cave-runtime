// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hubble observability — flow records, verdicts, drop reasons, filters.
//!
//! Ports cilium's observability layer:
//!   * The [`Flow`] record mirrors `flow.proto` (`api/v1/flow`): verdict,
//!     IP/L4/L7 tuple, source/destination [`Endpoint`], traffic direction.
//!   * [`DropReason`] is the full numeric drop-code table from
//!     `pkg/monitor/api/drop.go` (v1.19.4) — so a `DROPPED` flow's reason
//!     renders identically to `hubble observe`.
//!   * [`FlowFilter`] ports `pkg/hubble/filters`: fields are AND-ed within a
//!     filter, OR-ed within a field; include is OR-of-filters, exclude
//!     rejects.
//!   * [`FlowBuffer`] ports the lock-free ring (`pkg/hubble/container/ring`)
//!     as a bounded buffer that overwrites the oldest flow.

use std::collections::VecDeque;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

/// Forwarding verdict (`flow.Verdict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Unknown,
    Forwarded,
    Dropped,
    Error,
    Audit,
    Redirected,
    Traced,
    Translated,
}

/// Traffic direction (`flow.TrafficDirection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrafficDirection {
    Unknown,
    Ingress,
    Egress,
}

/// L4 protocol of the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum L4Protocol {
    Tcp,
    Udp,
    Icmpv4,
    Icmpv6,
    Sctp,
}

/// A flow endpoint (`flow.Endpoint`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub identity: u32,
    pub namespace: String,
    pub pod_name: String,
    pub labels: Vec<String>,
}

/// L7 sub-record (HTTP shown; cilium also models DNS/Kafka).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L7Flow {
    pub method: String,
    pub path: String,
    pub status: u32,
}

/// A single observed flow (`flow.Flow`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub time_ns: u64,
    pub verdict: Verdict,
    pub drop_reason: u8,
    pub ip_source: Ipv4Addr,
    pub ip_destination: Ipv4Addr,
    pub l4_protocol: L4Protocol,
    pub source_port: u16,
    pub destination_port: u16,
    pub source: Endpoint,
    pub destination: Endpoint,
    pub traffic_direction: TrafficDirection,
    pub l7: Option<L7Flow>,
}

impl Flow {
    pub fn is_dropped(&self) -> bool {
        matches!(self.verdict, Verdict::Dropped)
    }

    /// The human-readable drop reason, exactly as `hubble observe` prints it.
    pub fn drop_reason_desc(&self) -> &'static str {
        DropReason::name(self.drop_reason)
    }
}

/// The numeric drop-reason table (`pkg/monitor/api/drop.go`, v1.19.4).
pub struct DropReason;

impl DropReason {
    pub fn name(code: u8) -> &'static str {
        match code {
            0 => "Success",
            2 => "Invalid packet",
            3 => "Interface",
            4 => "Interface Decrypted",
            5 => "LB, sock cgroup: No backend slot entry found",
            6 => "LB, sock cgroup: No backend entry found",
            7 => "LB, sock cgroup: Reverse entry update failed",
            8 => "LB, sock cgroup: Reverse entry stale",
            9 => "Fragmented packet",
            10 => "Fragmented packet entry update failed",
            11 => "Missed tail call to custom program",
            12 => "Interface Decrypting",
            13 => "Interface Encrypting",
            14 => "LB: sock cgroup: Reverse entry delete succeeded",
            15 => "MTU error message",
            130 => "Invalid source mac",
            131 => "Invalid destination mac",
            132 => "Invalid source ip",
            133 => "Policy denied",
            134 => "Invalid packet",
            135 => "CT: Truncated or invalid header",
            136 => "Fragmentation needed",
            137 => "CT: Unknown L4 protocol",
            138 => "CT: Can't create entry from packet",
            139 => "Unsupported L3 protocol",
            140 => "Missed tail call",
            141 => "Error writing to packet",
            142 => "Unknown L4 protocol",
            143 => "Unknown ICMPv4 code",
            144 => "Unknown ICMPv4 type",
            145 => "Unknown ICMPv6 code",
            146 => "Unknown ICMPv6 type",
            147 => "Error retrieving tunnel key",
            148 => "Error retrieving tunnel options",
            149 => "Invalid Geneve option",
            150 => "Unknown L3 target address",
            151 => "Stale or unroutable IP",
            152 => "No matching local container found",
            153 => "Error while correcting L3 checksum",
            154 => "Error while correcting L4 checksum",
            155 => "CT: Map insertion failed",
            156 => "Invalid IPv6 extension header",
            157 => "IP fragmentation not supported",
            158 => "Service backend not found",
            160 => "No tunnel/encapsulation endpoint (datapath BUG!)",
            161 => "NAT 46/64 not enabled",
            162 => "Reached EDT rate-limiting drop horizon",
            163 => "Unknown connection tracking state",
            164 => "Local host is unreachable",
            165 => "No configuration available to perform policy decision",
            166 => "Unsupported L2 protocol",
            167 => "No mapping for NAT masquerade",
            168 => "Unsupported protocol for NAT masquerade",
            169 => "FIB lookup failed",
            170 => "Encapsulation traffic is prohibited",
            171 => "Invalid identity",
            172 => "Unknown sender",
            173 => "NAT not needed",
            174 => "Is a ClusterIP",
            175 => "First logical datagram fragment not found",
            176 => "Forbidden ICMPv6 message",
            177 => "Denied by LB src range check",
            178 => "Socket lookup failed",
            179 => "Socket assign failed",
            180 => "Proxy redirection not supported for protocol",
            181 => "Policy denied by denylist",
            182 => "VLAN traffic disallowed by VLAN filter",
            183 => "Incorrect VNI from VTEP",
            184 => "Failed to update or lookup TC buffer",
            185 => "No SID was found for the IP address",
            186 => "SRv6 state was removed during tail call",
            187 => "L3 translation from IPv4 to IPv6 failed (NAT46)",
            188 => "L3 translation from IPv6 to IPv4 failed (NAT64)",
            189 => "Authentication required",
            190 => "No conntrack map found",
            191 => "No nat map found",
            192 => "Invalid ClusterID",
            193 => "Unsupported packet protocol for DSR encapsulation",
            194 => "No egress gateway found",
            195 => "Traffic is unencrypted",
            196 => "TTL exceeded",
            197 => "No node ID found",
            198 => "Rate limited",
            199 => "IGMP handled",
            200 => "IGMP subscribed",
            201 => "Multicast handled",
            202 => "Host datapath not ready",
            203 => "Endpoint policy program not available",
            204 => "No Egress IP configured",
            205 => "Punt to proxy",
            _ => "Unknown",
        }
    }

    /// True for the policy-deny codes (133 / 181).
    pub fn is_policy_denied(code: u8) -> bool {
        matches!(code, 133 | 181)
    }
}

/// A Hubble flow filter (`pkg/hubble/filters`).
#[derive(Debug, Clone, Default)]
pub struct FlowFilter {
    pub source_identity: Vec<u32>,
    pub destination_identity: Vec<u32>,
    pub verdict: Vec<Verdict>,
    pub source_pod: Vec<String>,
    pub destination_pod: Vec<String>,
    pub source_namespace: Vec<String>,
    pub protocol: Vec<L4Protocol>,
}

impl FlowFilter {
    /// AND across populated fields; OR within each field's list. An empty
    /// field is a wildcard (does not constrain).
    pub fn matches(&self, f: &Flow) -> bool {
        let any = |list: &[u32], v: u32| list.is_empty() || list.contains(&v);
        let any_str = |list: &[String], v: &str| {
            list.is_empty() || list.iter().any(|s| s == v)
        };
        any(&self.source_identity, f.source.identity)
            && any(&self.destination_identity, f.destination.identity)
            && (self.verdict.is_empty() || self.verdict.contains(&f.verdict))
            && (self.protocol.is_empty() || self.protocol.contains(&f.l4_protocol))
            && any_str(&self.source_pod, &f.source.pod_name)
            && any_str(&self.destination_pod, &f.destination.pod_name)
            && any_str(&self.source_namespace, &f.source.namespace)
    }
}

/// A bounded ring buffer of recent flows (`pkg/hubble/container/ring`).
#[derive(Debug)]
pub struct FlowBuffer {
    cap: usize,
    flows: VecDeque<Flow>,
}

impl Default for FlowBuffer {
    fn default() -> Self {
        // cilium's ring is 2^N - 1 sized; the default agent capacity is 4095.
        FlowBuffer::with_capacity(4095)
    }
}

impl FlowBuffer {
    pub fn with_capacity(cap: usize) -> Self {
        FlowBuffer {
            cap: cap.max(1),
            flows: VecDeque::with_capacity(cap.max(1)),
        }
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.flows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }

    /// Append a flow, evicting the oldest if at capacity.
    pub fn push(&mut self, f: Flow) {
        if self.flows.len() == self.cap {
            self.flows.pop_front();
        }
        self.flows.push_back(f);
    }

    /// The `n` most recent flows, newest first.
    pub fn recent(&self, n: usize) -> Vec<&Flow> {
        self.flows.iter().rev().take(n).collect()
    }

    /// Query with Hubble include/exclude semantics: a flow passes if
    /// (include empty OR matches some include) AND (matches no exclude).
    pub fn query(&self, include: &[FlowFilter], exclude: &[FlowFilter]) -> Vec<&Flow> {
        self.flows
            .iter()
            .rev()
            .filter(|f| {
                let included = include.is_empty() || include.iter().any(|flt| flt.matches(f));
                let excluded = exclude.iter().any(|flt| flt.matches(f));
                included && !excluded
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn flow(verdict: Verdict, src_id: u32, dst_id: u32, ns: &str) -> Flow {
        Flow {
            time_ns: 0,
            verdict,
            drop_reason: 0,
            ip_source: Ipv4Addr::new(10, 0, 0, 1),
            ip_destination: Ipv4Addr::new(10, 0, 0, 2),
            l4_protocol: L4Protocol::Tcp,
            source_port: 12345,
            destination_port: 80,
            source: Endpoint {
                identity: src_id,
                namespace: ns.into(),
                pod_name: "client".into(),
                labels: vec![],
            },
            destination: Endpoint {
                identity: dst_id,
                namespace: ns.into(),
                pod_name: "web".into(),
                labels: vec![],
            },
            traffic_direction: TrafficDirection::Ingress,
            l7: None,
        }
    }

    #[test]
    fn drop_reason_table_matches_cilium() {
        assert_eq!(DropReason::name(0), "Success");
        assert_eq!(DropReason::name(133), "Policy denied");
        assert_eq!(DropReason::name(151), "Stale or unroutable IP");
        assert_eq!(DropReason::name(205), "Punt to proxy");
        assert_eq!(DropReason::name(250), "Unknown");
    }

    #[test]
    fn dropped_flow_carries_reason_description() {
        let mut f = flow(Verdict::Dropped, 256, 257, "default");
        f.drop_reason = 133;
        assert_eq!(f.drop_reason_desc(), "Policy denied");
        assert!(f.is_dropped());
    }

    #[test]
    fn ring_buffer_overwrites_oldest() {
        let mut b = FlowBuffer::with_capacity(3);
        for i in 0..5 {
            let mut f = flow(Verdict::Forwarded, 256, 257, "default");
            f.time_ns = i;
            b.push(f);
        }
        assert_eq!(b.len(), 3, "capacity is bounded");
        // recent() is newest-first; oldest two (0,1) were evicted.
        let recent = b.recent(3);
        assert_eq!(recent[0].time_ns, 4);
        assert_eq!(recent[2].time_ns, 2);
    }

    #[test]
    fn filter_ands_fields_within_a_filter() {
        let mut b = FlowBuffer::with_capacity(16);
        b.push(flow(Verdict::Dropped, 256, 257, "default"));
        b.push(flow(Verdict::Forwarded, 256, 257, "default"));
        b.push(flow(Verdict::Dropped, 999, 257, "kube-system"));

        // include: verdict=DROPPED AND source_identity=256
        let include = vec![FlowFilter {
            verdict: vec![Verdict::Dropped],
            source_identity: vec![256],
            ..Default::default()
        }];
        let got = b.query(&include, &[]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source.identity, 256);
        assert!(got[0].is_dropped());
    }

    #[test]
    fn exclude_filters_drop_matching_flows() {
        let mut b = FlowBuffer::with_capacity(16);
        b.push(flow(Verdict::Dropped, 256, 257, "default"));
        b.push(flow(Verdict::Dropped, 256, 257, "kube-system"));

        // No include → all pass, except excluded kube-system namespace.
        let exclude = vec![FlowFilter {
            source_namespace: vec!["kube-system".into()],
            ..Default::default()
        }];
        let got = b.query(&[], &exclude);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source.namespace, "default");
    }

    #[test]
    fn empty_include_passes_all() {
        let mut b = FlowBuffer::with_capacity(8);
        b.push(flow(Verdict::Forwarded, 256, 257, "default"));
        assert_eq!(b.query(&[], &[]).len(), 1);
    }

    #[test]
    fn default_buffer_has_power_of_two_minus_one_capacity() {
        let b = FlowBuffer::default();
        assert_eq!(b.capacity(), 4095);
    }
}

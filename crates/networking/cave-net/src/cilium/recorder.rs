// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Recorder — pcap-style flow capture via BPF perf events.
//!
//! Mirrors `pkg/recorder/recorder.go` plus the
//! `CiliumRecorderPolicy` CRD shape from
//! `pkg/k8s/apis/cilium.io/v2alpha1/types.go`. The recorder
//! samples packets matching a 5-tuple filter and emits them to the
//! perf-event ring for the user-space `cilium recorder` collector
//! to write as pcap.
//!
//! Semantics (faithful to upstream):
//!
//! * A `RecorderTuple` selects packets by 5-tuple wildcards
//!   (port = 0 → any, address mask 0 → any).
//! * Each tuple has a `capture_length` (max bytes per packet) and a
//!   `sample_one_in_n` rate (1 = capture every packet, 100 = ~1%).
//! * Multiple tuples can be active simultaneously; matching is
//!   first-match-wins ordered by priority.
//! * Captured packets are pushed to a shared perf-event ring keyed
//!   by recorder-id. We model the ring as a VecDeque with a max
//!   capacity (overflow drops the oldest, mirroring the kernel
//!   `BPF_MAP_TYPE_PERF_EVENT_ARRAY` behaviour).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecorderProto {
    Any,
    Tcp,
    Udp,
    Icmp,
}

impl RecorderProto {
    pub fn covers(self, wire: u8) -> bool {
        match self {
            RecorderProto::Any => true,
            RecorderProto::Tcp => wire == 6,
            RecorderProto::Udp => wire == 17,
            RecorderProto::Icmp => wire == 1 || wire == 58,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecorderTuple {
    pub src_ip: Option<IpAddr>,
    pub dst_ip: Option<IpAddr>,
    pub src_port: u16, // 0 = any
    pub dst_port: u16, // 0 = any
    pub protocol: RecorderProto,
}

impl RecorderTuple {
    pub fn matches(&self, src: IpAddr, dst: IpAddr, sp: u16, dp: u16, proto: u8) -> bool {
        if let Some(s) = self.src_ip {
            if s != src {
                return false;
            }
        }
        if let Some(d) = self.dst_ip {
            if d != dst {
                return false;
            }
        }
        if self.src_port != 0 && self.src_port != sp {
            return false;
        }
        if self.dst_port != 0 && self.dst_port != dp {
            return false;
        }
        self.protocol.covers(proto)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecorderPolicy {
    pub id: u32,
    pub priority: u32,
    pub tuple: RecorderTuple,
    pub capture_length: u16,
    pub sample_one_in_n: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedPacket {
    pub recorder_id: u32,
    pub timestamp_ns: u64,
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8,
    pub bytes: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecorderError {
    #[error("recorder id {0} already exists")]
    DuplicateId(u32),
    #[error("recorder id {0} not found")]
    NotFound(u32),
    #[error("sample rate `1 in {0}` is invalid (must be ≥ 1)")]
    BadSampleRate(u32),
    #[error("tenant {tenant} cannot mutate recorder owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct Recorder {
    pub tenant: TenantId,
    pub ring_capacity: usize,
    policies: HashMap<u32, RecorderPolicy>,
    /// Per-recorder packet counter (used for sampling).
    counters: HashMap<u32, u64>,
    /// Per-recorder ring buffer of captured packets.
    rings: HashMap<u32, VecDeque<CapturedPacket>>,
    /// Per-recorder overflow count.
    overflows: HashMap<u32, u64>,
}

impl Recorder {
    pub fn new(tenant: TenantId, ring_capacity: usize) -> Self {
        Self {
            tenant,
            ring_capacity,
            policies: HashMap::new(),
            counters: HashMap::new(),
            rings: HashMap::new(),
            overflows: HashMap::new(),
        }
    }

    pub fn upsert_policy(&mut self, p: RecorderPolicy) -> Result<(), RecorderError> {
        if p.sample_one_in_n == 0 {
            return Err(RecorderError::BadSampleRate(0));
        }
        self.policies.insert(p.id, p);
        Ok(())
    }

    pub fn remove_policy(&mut self, id: u32) -> Result<(), RecorderError> {
        self.policies
            .remove(&id)
            .ok_or(RecorderError::NotFound(id))?;
        self.counters.remove(&id);
        self.rings.remove(&id);
        self.overflows.remove(&id);
        Ok(())
    }

    pub fn policy_count(&self) -> usize {
        self.policies.len()
    }

    /// Try to capture a packet. Returns `Some(recorder_id)` when a
    /// policy matched AND the sample-rate gate fired, else `None`.
    pub fn capture(
        &mut self,
        src: IpAddr,
        dst: IpAddr,
        sp: u16,
        dp: u16,
        proto: u8,
        timestamp_ns: u64,
        bytes: &[u8],
    ) -> Option<u32> {
        // Sort policies by priority (highest first) for first-match-wins.
        let mut ordered: Vec<&RecorderPolicy> = self.policies.values().collect();
        ordered.sort_by(|a, b| b.priority.cmp(&a.priority));
        for p in ordered {
            if !p.tuple.matches(src, dst, sp, dp, proto) {
                continue;
            }
            // Sample gate.
            let counter = self.counters.entry(p.id).or_insert(0);
            let captured = *counter % p.sample_one_in_n as u64 == 0;
            *counter += 1;
            if !captured {
                return None;
            }
            // Truncate to capture length.
            let truncated = if bytes.len() > p.capture_length as usize {
                bytes[..p.capture_length as usize].to_vec()
            } else {
                bytes.to_vec()
            };
            let pkt = CapturedPacket {
                recorder_id: p.id,
                timestamp_ns,
                src_ip: src,
                dst_ip: dst,
                src_port: sp,
                dst_port: dp,
                protocol: proto,
                bytes: truncated,
            };
            let ring = self.rings.entry(p.id).or_insert_with(VecDeque::new);
            if ring.len() >= self.ring_capacity {
                ring.pop_front();
                *self.overflows.entry(p.id).or_insert(0) += 1;
            }
            ring.push_back(pkt);
            return Some(p.id);
        }
        None
    }

    pub fn drain(&mut self, id: u32) -> Result<Vec<CapturedPacket>, RecorderError> {
        if !self.policies.contains_key(&id) {
            return Err(RecorderError::NotFound(id));
        }
        let ring = self.rings.entry(id).or_insert_with(VecDeque::new);
        Ok(std::mem::take(ring).into_iter().collect())
    }

    pub fn ring_len(&self, id: u32) -> usize {
        self.rings.get(&id).map(|r| r.len()).unwrap_or(0)
    }

    pub fn overflow_count(&self, id: u32) -> u64 {
        self.overflows.get(&id).copied().unwrap_or(0)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/recorder/recorder.go", "Recorder");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn rec(tenant: TenantId, cap: usize) -> Recorder {
        Recorder::new(tenant, cap)
    }

    fn match_all_tuple() -> RecorderTuple {
        RecorderTuple {
            src_ip: None,
            dst_ip: None,
            src_port: 0,
            dst_port: 0,
            protocol: RecorderProto::Any,
        }
    }

    fn policy(id: u32, priority: u32, sample: u32) -> RecorderPolicy {
        RecorderPolicy {
            id,
            priority,
            tuple: match_all_tuple(),
            capture_length: 1500,
            sample_one_in_n: sample,
        }
    }

    // ── RecorderProto.covers ────────────────────────────────────────────────

    #[test]
    fn proto_any_covers_everything() {
        let (_c, _t) = cilium_test_ctx!("pkg/recorder/recorder.go", "Proto.Any", "tenant-rec-pa");
        assert!(RecorderProto::Any.covers(6));
        assert!(RecorderProto::Any.covers(17));
        assert!(RecorderProto::Any.covers(1));
    }

    #[test]
    fn proto_tcp_covers_only_6() {
        let (_c, _t) = cilium_test_ctx!("pkg/recorder/recorder.go", "Proto.TCP", "tenant-rec-tcp");
        assert!(RecorderProto::Tcp.covers(6));
        assert!(!RecorderProto::Tcp.covers(17));
    }

    #[test]
    fn proto_icmp_covers_v4_and_v6_codes() {
        let (_c, _t) =
            cilium_test_ctx!("pkg/recorder/recorder.go", "Proto.ICMP", "tenant-rec-icmp");
        assert!(RecorderProto::Icmp.covers(1)); // ICMPv4
        assert!(RecorderProto::Icmp.covers(58)); // ICMPv6
        assert!(!RecorderProto::Icmp.covers(6));
    }

    // ── RecorderTuple.matches ───────────────────────────────────────────────

    #[test]
    fn tuple_matches_all_when_all_wildcards() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Tuple.MatchAll",
            "tenant-rec-tma"
        );
        let t = match_all_tuple();
        assert!(t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6));
    }

    #[test]
    fn tuple_filters_by_src_port() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Tuple.SrcPort",
            "tenant-rec-tsp"
        );
        let mut t = match_all_tuple();
        t.src_port = 1234;
        assert!(t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6));
        assert!(!t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 5555, 80, 6));
    }

    #[test]
    fn tuple_filters_by_dst_port() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Tuple.DstPort",
            "tenant-rec-tdp"
        );
        let mut t = match_all_tuple();
        t.dst_port = 80;
        assert!(t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6));
        assert!(!t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 443, 6));
    }

    #[test]
    fn tuple_filters_by_dst_ip() {
        let (_c, _t) =
            cilium_test_ctx!("pkg/recorder/recorder.go", "Tuple.DstIP", "tenant-rec-tdi");
        let mut t = match_all_tuple();
        t.dst_ip = Some(ip(10, 0, 0, 2));
        assert!(t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6));
        assert!(!t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 9), 1234, 80, 6));
    }

    #[test]
    fn tuple_filters_by_protocol() {
        let (_c, _t) =
            cilium_test_ctx!("pkg/recorder/recorder.go", "Tuple.Proto", "tenant-rec-tpr");
        let mut t = match_all_tuple();
        t.protocol = RecorderProto::Tcp;
        assert!(t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6));
        assert!(!t.matches(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 17));
    }

    // ── Recorder upsert / remove ────────────────────────────────────────────

    #[test]
    fn recorder_upsert_policy() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Recorder.Upsert",
            "tenant-rec-up"
        );
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        assert_eq!(r.policy_count(), 1);
    }

    #[test]
    fn recorder_upsert_with_zero_sample_rate_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Recorder.Upsert.BadRate",
            "tenant-rec-uprate"
        );
        let mut r = rec(tenant, 100);
        let err = r.upsert_policy(policy(1, 10, 0)).unwrap_err();
        assert_eq!(err, RecorderError::BadSampleRate(0));
    }

    #[test]
    fn recorder_remove_policy() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Recorder.Remove",
            "tenant-rec-rm"
        );
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        r.remove_policy(1).unwrap();
        assert_eq!(r.policy_count(), 0);
    }

    #[test]
    fn recorder_remove_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Recorder.Remove.NotFound",
            "tenant-rec-rmnf"
        );
        let mut r = rec(tenant, 100);
        let err = r.remove_policy(1).unwrap_err();
        assert_eq!(err, RecorderError::NotFound(1));
    }

    // ── Capture ────────────────────────────────────────────────────────────

    #[test]
    fn capture_first_packet_records() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/recorder/recorder.go", "Capture.First", "tenant-rec-c1");
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        let id = r
            .capture(
                ip(10, 0, 0, 1),
                ip(10, 0, 0, 2),
                1234,
                80,
                6,
                100,
                &[0u8; 64],
            )
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(r.ring_len(1), 1);
    }

    #[test]
    fn capture_no_matching_policy_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Capture.NoMatch",
            "tenant-rec-cnm"
        );
        let mut r = rec(tenant, 100);
        let mut p = policy(1, 10, 1);
        p.tuple.dst_port = 443;
        r.upsert_policy(p).unwrap();
        assert!(r
            .capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, 100, &[0; 32])
            .is_none());
        assert_eq!(r.ring_len(1), 0);
    }

    #[test]
    fn capture_truncates_to_capture_length() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Capture.Truncate",
            "tenant-rec-ctr"
        );
        let mut r = rec(tenant, 100);
        let mut p = policy(1, 10, 1);
        p.capture_length = 64;
        r.upsert_policy(p).unwrap();
        r.capture(
            ip(10, 0, 0, 1),
            ip(10, 0, 0, 2),
            1234,
            80,
            6,
            100,
            &[0xAB; 1500],
        );
        let pkts = r.drain(1).unwrap();
        assert_eq!(pkts[0].bytes.len(), 64);
    }

    #[test]
    fn capture_keeps_full_payload_when_smaller_than_capture_length() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Capture.Untruncated",
            "tenant-rec-cunt"
        );
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        r.capture(
            ip(10, 0, 0, 1),
            ip(10, 0, 0, 2),
            1234,
            80,
            6,
            100,
            &[1u8; 100],
        );
        let pkts = r.drain(1).unwrap();
        assert_eq!(pkts[0].bytes.len(), 100);
    }

    // ── Sampling ───────────────────────────────────────────────────────────

    #[test]
    fn capture_samples_every_packet_when_rate_1() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Capture.Sample.AllPackets",
            "tenant-rec-samp1"
        );
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        for _ in 0..10 {
            r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, 0, &[0; 64]);
        }
        assert_eq!(r.ring_len(1), 10);
    }

    #[test]
    fn capture_samples_one_in_three_at_rate_3() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Capture.Sample.OneInThree",
            "tenant-rec-samp3"
        );
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 3)).unwrap();
        for _ in 0..9 {
            r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, 0, &[0; 64]);
        }
        // Counter increments 1..9; captured at counter=1, 4, 7 → 3 packets.
        assert_eq!(r.ring_len(1), 3);
    }

    // ── Priority ───────────────────────────────────────────────────────────

    #[test]
    fn capture_first_match_wins_by_priority() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Capture.Priority",
            "tenant-rec-pri"
        );
        let mut r = rec(tenant, 100);
        let mut low = policy(1, 5, 1);
        low.capture_length = 32;
        let mut high = policy(2, 100, 1);
        high.capture_length = 256;
        r.upsert_policy(low).unwrap();
        r.upsert_policy(high).unwrap();
        let id = r
            .capture(
                ip(10, 0, 0, 1),
                ip(10, 0, 0, 2),
                1234,
                80,
                6,
                100,
                &[0; 1500],
            )
            .unwrap();
        // Higher priority wins.
        assert_eq!(id, 2);
        let pkts = r.drain(2).unwrap();
        assert_eq!(pkts[0].bytes.len(), 256);
    }

    // ── Ring overflow ──────────────────────────────────────────────────────

    #[test]
    fn ring_drops_oldest_when_full() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/recorder/recorder.go", "Ring.Overflow", "tenant-rec-ov");
        let mut r = rec(tenant, 3);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        for _ in 0..5 {
            r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, 0, &[0; 64]);
        }
        assert_eq!(r.ring_len(1), 3);
        assert_eq!(r.overflow_count(1), 2);
    }

    // ── Drain ──────────────────────────────────────────────────────────────

    #[test]
    fn drain_returns_captured_in_fifo_order() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/recorder/recorder.go", "Drain.FIFO", "tenant-rec-dfifo");
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        for ts in 0..5u64 {
            r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, ts, &[0; 64]);
        }
        let pkts = r.drain(1).unwrap();
        assert_eq!(pkts.len(), 5);
        for (i, p) in pkts.iter().enumerate() {
            assert_eq!(p.timestamp_ns, i as u64);
        }
    }

    #[test]
    fn drain_clears_ring() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Drain.Clears",
            "tenant-rec-dclr"
        );
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, 0, &[0; 64]);
        let _ = r.drain(1).unwrap();
        assert_eq!(r.ring_len(1), 0);
    }

    #[test]
    fn drain_unknown_recorder_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Drain.NotFound",
            "tenant-rec-dnf"
        );
        let mut r = rec(tenant, 100);
        let err = r.drain(99).unwrap_err();
        assert_eq!(err, RecorderError::NotFound(99));
    }

    // ── Lifecycle ──────────────────────────────────────────────────────────

    #[test]
    fn remove_policy_drops_ring_and_counters() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Remove.DropsRing",
            "tenant-rec-rmr"
        );
        let mut r = rec(tenant, 100);
        r.upsert_policy(policy(1, 10, 1)).unwrap();
        r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, 0, &[0; 64]);
        r.remove_policy(1).unwrap();
        // Drain after remove should error.
        assert!(matches!(
            r.drain(1).unwrap_err(),
            RecorderError::NotFound(1)
        ));
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn recorder_policy_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Policy.Serde",
            "tenant-rec-pserde"
        );
        let p = policy(1, 10, 1);
        let s = serde_json::to_string(&p).unwrap();
        let back: RecorderPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn captured_packet_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Captured.Serde",
            "tenant-rec-cserde"
        );
        let p = CapturedPacket {
            recorder_id: 1,
            timestamp_ns: 100,
            src_ip: ip(10, 0, 0, 1),
            dst_ip: ip(10, 0, 0, 2),
            src_port: 1234,
            dst_port: 80,
            protocol: 6,
            bytes: vec![1, 2, 3, 4],
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: CapturedPacket = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn recorder_proto_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Proto.Serde",
            "tenant-rec-prtserde"
        );
        for p in [
            RecorderProto::Any,
            RecorderProto::Tcp,
            RecorderProto::Udp,
            RecorderProto::Icmp,
        ] {
            let s = serde_json::to_string(&p).unwrap();
            let back: RecorderProto = serde_json::from_str(&s).unwrap();
            assert_eq!(back, p);
        }
    }

    // ── Multi-recorder ──────────────────────────────────────────────────────

    #[test]
    fn multiple_recorders_independent_rings() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/recorder/recorder.go",
            "Multi.IndependentRings",
            "tenant-rec-multi"
        );
        let mut r = rec(tenant, 100);
        let mut p1 = policy(1, 100, 1);
        p1.tuple.dst_port = 80;
        let mut p2 = policy(2, 100, 1);
        p2.tuple.dst_port = 443;
        r.upsert_policy(p1).unwrap();
        r.upsert_policy(p2).unwrap();
        r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 80, 6, 0, &[0; 64]);
        r.capture(ip(10, 0, 0, 1), ip(10, 0, 0, 2), 1234, 443, 6, 0, &[0; 64]);
        // Each recorder has 1 packet — but priority is equal so first inserted wins;
        // Map iteration order is non-deterministic, so just check totals.
        assert_eq!(r.ring_len(1) + r.ring_len(2), 2);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bandwidth manager — per-pod rate limiting via EDT (Earliest Departure
//! Time) and BBR congestion-control hand-off.
//!
//! Mirrors `pkg/datapath/linux/bandwidth/bandwidth.go` (the cilium-agent
//! manager) and the `bpf/bpf_host.c::throttle_egress` BPF helper.
//!
//! Semantics (faithful to upstream):
//!
//! * Per-pod bandwidth annotation `kubernetes.io/egress-bandwidth` is
//!   converted to bytes-per-second and stored in a BPF hash map keyed
//!   by endpoint id.
//! * The egress program calculates an EDT (`pkt->tstamp + delay`)
//!   based on the remaining quantum so packets arrive at the kernel
//!   FQ qdisc with a future timestamp; FQ then paces them.
//! * BBR (TCP_CONGESTION = "bbr") relies on the same FQ pacing —
//!   when bandwidth manager + BBR are both enabled, the EDT
//!   timestamps double as BBR's "bw + min_rtt" probe input.
//! * Bandwidth removal sets the limit to 0 (unlimited).
//! * Token bucket: each endpoint keeps `tokens` (bytes available
//!   immediately) and `rate` (bytes/s); on each packet arrival
//!   `tokens` is replenished by `rate * elapsed_ns / 1e9`, capped at
//!   `burst` (= `rate / 4` upstream default).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BandwidthError {
    #[error("invalid k8s bandwidth annotation `{0}`")]
    BadAnnotation(String),
    #[error("endpoint id {0} not registered")]
    EndpointNotRegistered(u64),
    #[error("tenant {tenant} cannot mutate bandwidth manager owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Parse the K8s `kubernetes.io/egress-bandwidth` annotation. Accepts
/// the SI suffixes Kubernetes documents (`K`, `M`, `G`, `T` for SI
/// kilobits, `Ki`, `Mi`, `Gi`, `Ti` for binary). Returns bytes/sec.
pub fn parse_bandwidth_annotation(s: &str) -> Result<u64, BandwidthError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(BandwidthError::BadAnnotation(s.to_string()));
    }
    let (num_str, mult) = if let Some(stripped) = s.strip_suffix("Ti") {
        (stripped, 1u64 << 40)
    } else if let Some(stripped) = s.strip_suffix("Gi") {
        (stripped, 1u64 << 30)
    } else if let Some(stripped) = s.strip_suffix("Mi") {
        (stripped, 1u64 << 20)
    } else if let Some(stripped) = s.strip_suffix("Ki") {
        (stripped, 1u64 << 10)
    } else if let Some(stripped) = s.strip_suffix('T') {
        (stripped, 1_000_000_000_000u64)
    } else if let Some(stripped) = s.strip_suffix('G') {
        (stripped, 1_000_000_000u64)
    } else if let Some(stripped) = s.strip_suffix('M') {
        (stripped, 1_000_000u64)
    } else if let Some(stripped) = s.strip_suffix('K') {
        (stripped, 1_000u64)
    } else {
        (s, 1u64)
    };
    let num: u64 = num_str.parse().map_err(|_| BandwidthError::BadAnnotation(s.to_string()))?;
    num.checked_mul(mult).ok_or_else(|| BandwidthError::BadAnnotation(s.to_string()))
}

/// Per-endpoint bandwidth entry. Mirrors
/// `bpf/lib/bandwidth.h::edt_info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BandwidthEntry {
    pub endpoint_id: u64,
    /// Rate in bytes/sec. 0 = unlimited.
    pub rate_bps: u64,
    /// Burst size in bytes. Default = rate_bps / 4 (upstream).
    pub burst_bytes: u64,
    pub tokens: i64,
    pub last_update_ns: u64,
}

impl BandwidthEntry {
    pub fn new(endpoint_id: u64, rate_bps: u64) -> Self {
        let burst = if rate_bps > 0 { rate_bps / 4 } else { 0 };
        Self {
            endpoint_id, rate_bps, burst_bytes: burst,
            tokens: burst as i64,
            last_update_ns: 0,
        }
    }

    /// Replenish tokens based on elapsed time, cap at burst.
    fn refill(&mut self, now_ns: u64) {
        let elapsed_ns = now_ns.saturating_sub(self.last_update_ns);
        if elapsed_ns == 0 {
            return;
        }
        let added = (self.rate_bps as u128 * elapsed_ns as u128 / 1_000_000_000u128) as i64;
        self.tokens = (self.tokens + added).min(self.burst_bytes as i64);
        self.last_update_ns = now_ns;
    }

    /// Compute the EDT (Earliest Departure Time) for a packet of
    /// `pkt_bytes`. Returns `(edt_ns, accepted)`. If the token bucket
    /// has enough tokens, edt = now_ns and tokens are consumed; else
    /// edt is shifted into the future to space the packet.
    pub fn edt_for(&mut self, now_ns: u64, pkt_bytes: u64) -> (u64, bool) {
        if self.rate_bps == 0 {
            return (now_ns, true);
        }
        self.refill(now_ns);
        let pkt = pkt_bytes as i64;
        if self.tokens >= pkt {
            self.tokens -= pkt;
            (now_ns, true)
        } else {
            // Shortfall (positive number of bytes still needed).
            let needed_bytes = (pkt - self.tokens).max(0) as u128;
            let delay_ns = (needed_bytes * 1_000_000_000u128 / self.rate_bps as u128) as u64;
            self.tokens -= pkt;
            (now_ns + delay_ns, true)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CongestionControl {
    Cubic,
    Bbr,
    BbrV2,
    Reno,
}

impl CongestionControl {
    /// True if the algorithm is pacing-aware (i.e. relies on FQ + EDT
    /// timestamps). Mirrors `pkg/option/config.go::EnableBBRHostRouting`.
    pub fn requires_pacing(self) -> bool {
        matches!(self, CongestionControl::Bbr | CongestionControl::BbrV2)
    }
}

#[derive(Debug)]
pub struct BandwidthManager {
    pub tenant: TenantId,
    pub congestion: CongestionControl,
    /// Per-endpoint state.
    pub entries: HashMap<u64, BandwidthEntry>,
}

impl BandwidthManager {
    pub fn new(tenant: TenantId, congestion: CongestionControl) -> Self {
        Self { tenant, congestion, entries: HashMap::new() }
    }

    pub fn set_bandwidth(&mut self, endpoint_id: u64, rate_bps: u64) {
        let entry = BandwidthEntry::new(endpoint_id, rate_bps);
        self.entries.insert(endpoint_id, entry);
    }

    pub fn set_bandwidth_from_annotation(&mut self, endpoint_id: u64, annotation: &str) -> Result<(), BandwidthError> {
        let bps = parse_bandwidth_annotation(annotation)?;
        self.set_bandwidth(endpoint_id, bps);
        Ok(())
    }

    pub fn remove(&mut self, endpoint_id: u64) -> bool {
        self.entries.remove(&endpoint_id).is_some()
    }

    pub fn lookup(&self, endpoint_id: u64) -> Option<&BandwidthEntry> {
        self.entries.get(&endpoint_id)
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Compute EDT for a packet leaving `endpoint_id`. Returns the
    /// scheduled departure time in ns. If no entry exists, returns
    /// `now_ns` (no shaping).
    pub fn schedule(&mut self, endpoint_id: u64, now_ns: u64, pkt_bytes: u64) -> u64 {
        match self.entries.get_mut(&endpoint_id) {
            Some(e) => e.edt_for(now_ns, pkt_bytes).0,
            None => now_ns,
        }
    }

    /// True iff bandwidth manager + BBR are both enabled. Mirrors
    /// `pkg/option/config.go::EnableBBRHostRouting`.
    pub fn bbr_enabled(&self) -> bool {
        self.congestion.requires_pacing() && self.entries.values().any(|e| e.rate_bps > 0)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn mgr(tenant: TenantId) -> BandwidthManager {
        BandwidthManager::new(tenant, CongestionControl::Bbr)
    }

    // ── Annotation parsing ───────────────────────────────────────────────────

    #[test]
    fn parse_bw_plain_bytes() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "ParseBandwidth.Plain", "tenant-bw-plain");
        assert_eq!(parse_bandwidth_annotation("1000").unwrap(), 1000);
    }

    #[test]
    fn parse_bw_si_kilo_mega_giga() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "ParseBandwidth.SI", "tenant-bw-si");
        assert_eq!(parse_bandwidth_annotation("10K").unwrap(), 10_000);
        assert_eq!(parse_bandwidth_annotation("10M").unwrap(), 10_000_000);
        assert_eq!(parse_bandwidth_annotation("1G").unwrap(), 1_000_000_000);
    }

    #[test]
    fn parse_bw_binary_kibibyte_mebibyte_gibibyte() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "ParseBandwidth.Binary", "tenant-bw-bin");
        assert_eq!(parse_bandwidth_annotation("1Ki").unwrap(), 1024);
        assert_eq!(parse_bandwidth_annotation("1Mi").unwrap(), 1024 * 1024);
        assert_eq!(parse_bandwidth_annotation("1Gi").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn parse_bw_invalid_annotation_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "ParseBandwidth.Invalid", "tenant-bw-bad");
        let err = parse_bandwidth_annotation("garbage").unwrap_err();
        assert!(matches!(err, BandwidthError::BadAnnotation(_)));
    }

    #[test]
    fn parse_bw_empty_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "ParseBandwidth.Empty", "tenant-bw-empty");
        let err = parse_bandwidth_annotation("").unwrap_err();
        assert!(matches!(err, BandwidthError::BadAnnotation(_)));
    }

    #[test]
    fn parse_bw_terabyte() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "ParseBandwidth.T", "tenant-bw-t");
        assert_eq!(parse_bandwidth_annotation("1T").unwrap(), 1_000_000_000_000);
        assert_eq!(parse_bandwidth_annotation("1Ti").unwrap(), 1u64 << 40);
    }

    // ── BandwidthEntry token bucket ─────────────────────────────────────────

    #[test]
    fn bw_entry_unlimited_rate_returns_now() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/bandwidth.h", "edt.Unlimited", "tenant-bw-unl");
        let mut e = BandwidthEntry::new(1, 0);
        let (edt, accepted) = e.edt_for(1000, 1500);
        assert_eq!(edt, 1000);
        assert!(accepted);
    }

    #[test]
    fn bw_entry_burst_default_is_quarter_of_rate() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/bandwidth.h", "edt.Burst", "tenant-bw-burst");
        let e = BandwidthEntry::new(1, 1_000_000);
        assert_eq!(e.burst_bytes, 250_000);
    }

    #[test]
    fn bw_entry_packet_within_tokens_consumes_immediately() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/bandwidth.h", "edt.WithinBudget", "tenant-bw-within");
        let mut e = BandwidthEntry::new(1, 10_000_000); // 10 MB/s, burst 2.5 MB
        let (edt, _) = e.edt_for(1_000_000_000, 1500);
        assert_eq!(edt, 1_000_000_000);
        assert!(e.tokens < 2_500_000);
    }

    #[test]
    fn bw_entry_packet_exceeding_tokens_gets_future_edt() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/bandwidth.h", "edt.OverBudget", "tenant-bw-over");
        let mut e = BandwidthEntry::new(1, 1000); // 1000 B/s, burst 250 B
        // Send a 10 KB packet at t=0 — only 250 tokens available, need 10000.
        let (edt, accepted) = e.edt_for(0, 10_000);
        assert!(accepted);
        // Need ~9750 extra bytes at 1000 B/s = 9.75 seconds = 9_750_000_000 ns.
        assert!(edt >= 9_500_000_000);
        assert!(edt <= 10_000_000_000);
    }

    #[test]
    fn bw_entry_refills_tokens_over_time() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/bandwidth.h", "edt.Refill", "tenant-bw-refill");
        let mut e = BandwidthEntry::new(1, 1_000_000); // 1 MB/s
        e.tokens = 0;
        e.last_update_ns = 0;
        // After 1 second we should have refilled to burst (250 KB).
        let _ = e.edt_for(1_000_000_000, 0);
        assert!(e.tokens > 0);
        assert!(e.tokens <= e.burst_bytes as i64);
    }

    #[test]
    fn bw_entry_tokens_capped_at_burst() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/bandwidth.h", "edt.RefillCap", "tenant-bw-refillcap");
        let mut e = BandwidthEntry::new(1, 1_000_000); // burst 250000
        e.tokens = 100;
        e.last_update_ns = 0;
        // Ten seconds should over-saturate; cap at burst.
        let _ = e.edt_for(10_000_000_000, 0);
        assert_eq!(e.tokens, e.burst_bytes as i64);
    }

    // ── BandwidthManager ────────────────────────────────────────────────────

    #[test]
    fn bw_manager_set_and_lookup() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager.Set", "tenant-bw-set");
        let mut m = mgr(tenant);
        m.set_bandwidth(7, 1_000_000);
        let e = m.lookup(7).unwrap();
        assert_eq!(e.rate_bps, 1_000_000);
    }

    #[test]
    fn bw_manager_set_from_annotation() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager.SetFromAnnotation", "tenant-bw-ann");
        let mut m = mgr(tenant);
        m.set_bandwidth_from_annotation(7, "10M").unwrap();
        assert_eq!(m.lookup(7).unwrap().rate_bps, 10_000_000);
    }

    #[test]
    fn bw_manager_set_from_bad_annotation_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager.SetFromAnnotation.Bad", "tenant-bw-annbad");
        let mut m = mgr(tenant);
        assert!(m.set_bandwidth_from_annotation(7, "nope").is_err());
    }

    #[test]
    fn bw_manager_remove_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager.Delete", "tenant-bw-rm");
        let mut m = mgr(tenant);
        m.set_bandwidth(7, 1000);
        assert!(m.remove(7));
        assert!(m.lookup(7).is_none());
    }

    #[test]
    fn bw_manager_remove_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager.Delete.NotFound", "tenant-bw-rmnf");
        let mut m = mgr(tenant);
        assert!(!m.remove(999));
    }

    #[test]
    fn bw_manager_schedule_unknown_endpoint_returns_now() {
        let (_c, tenant) = cilium_test_ctx!("bpf/bpf_host.c", "throttle_egress.NoEntry", "tenant-bw-sched-nf");
        let mut m = mgr(tenant);
        let now = 1_000_000_000;
        assert_eq!(m.schedule(999, now, 1500), now);
    }

    #[test]
    fn bw_manager_schedule_within_budget_returns_now() {
        let (_c, tenant) = cilium_test_ctx!("bpf/bpf_host.c", "throttle_egress.Now", "tenant-bw-sched-now");
        let mut m = mgr(tenant);
        m.set_bandwidth(7, 10_000_000);
        let now = 1_000_000_000;
        assert_eq!(m.schedule(7, now, 1500), now);
    }

    #[test]
    fn bw_manager_schedule_exceeding_budget_returns_future() {
        let (_c, tenant) = cilium_test_ctx!("bpf/bpf_host.c", "throttle_egress.Future", "tenant-bw-sched-fut");
        let mut m = mgr(tenant);
        m.set_bandwidth(7, 1000);
        let now = 0;
        let edt = m.schedule(7, now, 10_000);
        assert!(edt > now);
    }

    #[test]
    fn bw_manager_entry_count_tracks_sets() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager.Len", "tenant-bw-len");
        let mut m = mgr(tenant);
        for i in 0..5u64 {
            m.set_bandwidth(i, 1_000_000);
        }
        assert_eq!(m.entry_count(), 5);
    }

    #[test]
    fn bw_manager_set_replaces_existing_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/bandwidth/bandwidth.go", "Manager.Set.Replace", "tenant-bw-rep");
        let mut m = mgr(tenant);
        m.set_bandwidth(7, 1000);
        m.set_bandwidth(7, 2000);
        assert_eq!(m.lookup(7).unwrap().rate_bps, 2000);
        assert_eq!(m.entry_count(), 1);
    }

    // ── Congestion control ──────────────────────────────────────────────────

    #[test]
    fn cc_bbr_requires_pacing() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "CongestionControl.BBR", "tenant-bw-cc-bbr");
        assert!(CongestionControl::Bbr.requires_pacing());
        assert!(CongestionControl::BbrV2.requires_pacing());
    }

    #[test]
    fn cc_cubic_does_not_require_pacing() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "CongestionControl.Cubic", "tenant-bw-cc-cubic");
        assert!(!CongestionControl::Cubic.requires_pacing());
        assert!(!CongestionControl::Reno.requires_pacing());
    }

    #[test]
    fn bw_manager_bbr_enabled_when_bbr_and_entries_present() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/config.go", "EnableBBRHostRouting", "tenant-bw-bbren");
        let mut m = BandwidthManager::new(tenant, CongestionControl::Bbr);
        m.set_bandwidth(7, 1_000_000);
        assert!(m.bbr_enabled());
    }

    #[test]
    fn bw_manager_bbr_disabled_when_no_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/config.go", "EnableBBRHostRouting.Empty", "tenant-bw-bbroff");
        let m = BandwidthManager::new(tenant, CongestionControl::Bbr);
        assert!(!m.bbr_enabled());
    }

    #[test]
    fn bw_manager_bbr_disabled_with_cubic_cc() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/config.go", "EnableBBRHostRouting.Cubic", "tenant-bw-bbrcubic");
        let mut m = BandwidthManager::new(tenant, CongestionControl::Cubic);
        m.set_bandwidth(7, 1_000_000);
        assert!(!m.bbr_enabled());
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn bw_entry_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/bandwidth.h", "edt_info.Serde", "tenant-bw-serde");
        let e = BandwidthEntry::new(7, 1_000_000);
        let json = serde_json::to_string(&e).unwrap();
        let back: BandwidthEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn cc_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "CongestionControl.Serde", "tenant-bw-ccserde");
        for c in [CongestionControl::Cubic, CongestionControl::Bbr, CongestionControl::BbrV2, CongestionControl::Reno] {
            let s = serde_json::to_string(&c).unwrap();
            let back: CongestionControl = serde_json::from_str(&s).unwrap();
            assert_eq!(back, c);
        }
    }

    // ── Realistic pacing scenario ───────────────────────────────────────────

    #[test]
    fn bw_pacing_distributes_packets_over_time() {
        let (_c, _t) = cilium_test_ctx!("bpf/bpf_host.c", "throttle_egress.Pacing", "tenant-bw-pace");
        let mut e = BandwidthEntry::new(1, 1_000); // 1000 B/s
        let mut last_edt = 0u64;
        for _ in 0..10 {
            let (edt, _) = e.edt_for(0, 200);
            // Each subsequent packet's EDT should be strictly later.
            assert!(edt >= last_edt);
            last_edt = edt;
        }
        // The last packet should be far in the future.
        assert!(last_edt > 1_000_000_000);
    }

    #[test]
    fn bw_pacing_zero_size_packet_does_not_advance() {
        let (_c, _t) = cilium_test_ctx!("bpf/bpf_host.c", "throttle_egress.Zero", "tenant-bw-pace0");
        let mut e = BandwidthEntry::new(1, 1_000_000);
        let (edt, _) = e.edt_for(1000, 0);
        assert_eq!(edt, 1000);
    }
}

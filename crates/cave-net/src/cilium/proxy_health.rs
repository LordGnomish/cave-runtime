// SPDX-License-Identifier: AGPL-3.0-or-later
//! Proxy health checks — agent ↔ envoy readiness probe.
//!
//! Mirrors `pkg/proxy/healthcheck.go`. The agent polls the envoy
//! admin `/ready` endpoint, tracks the last success / failure, and
//! flips the `ProxyState` between `Live`, `Degraded`, and `Down`.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyState {
    /// Last probe succeeded.
    Live,
    /// At least one probe failed but we haven't crossed the failure
    /// threshold for `Down`.
    Degraded,
    /// Failure threshold crossed; the proxy is considered down.
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyProbe {
    pub timestamp_ns: u64,
    pub success: bool,
    pub status_code: u16,
    pub latency_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyStatus {
    pub state: ProxyState,
    pub last_success_ns: u64,
    pub last_failure_ns: u64,
    pub consecutive_failures: u32,
    pub probes_total: u64,
    pub probes_success: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HealthError {
    #[error("proxy `{0}` not registered")]
    NotRegistered(String),
    #[error("tenant {tenant} cannot mutate proxy health owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct ProxyHealthChecker {
    pub tenant: TenantId,
    pub down_threshold: u32,
    proxies: BTreeMap<String, ProxyStatus>,
}

impl ProxyHealthChecker {
    pub fn new(tenant: TenantId, down_threshold: u32) -> Self {
        Self { tenant, down_threshold, proxies: BTreeMap::new() }
    }

    pub fn register(&mut self, name: impl Into<String>) {
        self.proxies.entry(name.into()).or_insert(ProxyStatus {
            state: ProxyState::Live,
            last_success_ns: 0, last_failure_ns: 0,
            consecutive_failures: 0,
            probes_total: 0, probes_success: 0,
        });
    }

    pub fn unregister(&mut self, name: &str) -> Result<(), HealthError> {
        self.proxies.remove(name).ok_or_else(|| HealthError::NotRegistered(name.to_string()))?;
        Ok(())
    }

    pub fn record(&mut self, name: &str, probe: ProxyProbe) -> Result<(), HealthError> {
        let s = self.proxies.get_mut(name).ok_or_else(|| HealthError::NotRegistered(name.to_string()))?;
        s.probes_total += 1;
        if probe.success {
            s.last_success_ns = probe.timestamp_ns;
            s.probes_success += 1;
            s.consecutive_failures = 0;
            s.state = ProxyState::Live;
        } else {
            s.last_failure_ns = probe.timestamp_ns;
            s.consecutive_failures += 1;
            s.state = if s.consecutive_failures >= self.down_threshold {
                ProxyState::Down
            } else {
                ProxyState::Degraded
            };
        }
        Ok(())
    }

    pub fn status(&self, name: &str) -> Option<&ProxyStatus> {
        self.proxies.get(name)
    }

    pub fn count(&self) -> usize {
        self.proxies.len()
    }

    pub fn live_count(&self) -> usize {
        self.proxies.values().filter(|s| matches!(s.state, ProxyState::Live)).count()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/proxy/healthcheck.go", "Checker");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn checker(tenant: TenantId) -> ProxyHealthChecker {
        ProxyHealthChecker::new(tenant, 3)
    }

    fn probe(success: bool, status: u16, ts: u64) -> ProxyProbe {
        ProxyProbe { timestamp_ns: ts, success, status_code: status, latency_us: 100 }
    }

    // ── Register / unregister ──────────────────────────────────────────────

    #[test]
    fn register_starts_in_live_state() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Register", "tenant-ph-r");
        let mut c = checker(tenant);
        c.register("envoy-1");
        assert_eq!(c.status("envoy-1").unwrap().state, ProxyState::Live);
    }

    #[test]
    fn unregister_drops_proxy() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Unregister", "tenant-ph-u");
        let mut c = checker(tenant);
        c.register("envoy-1");
        c.unregister("envoy-1").unwrap();
        assert_eq!(c.count(), 0);
    }

    #[test]
    fn unregister_unknown_returns_not_registered() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Unregister.NotFound", "tenant-ph-unf");
        let mut c = checker(tenant);
        let err = c.unregister("ghost").unwrap_err();
        assert!(matches!(err, HealthError::NotRegistered(_)));
    }

    // ── Record ────────────────────────────────────────────────────────────

    #[test]
    fn success_probe_keeps_state_live() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Record.Success", "tenant-ph-rs");
        let mut c = checker(tenant);
        c.register("envoy-1");
        c.record("envoy-1", probe(true, 200, 100)).unwrap();
        let s = c.status("envoy-1").unwrap();
        assert_eq!(s.state, ProxyState::Live);
        assert_eq!(s.probes_success, 1);
        assert_eq!(s.last_success_ns, 100);
    }

    #[test]
    fn first_failure_moves_state_to_degraded() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Record.Degraded", "tenant-ph-rd");
        let mut c = checker(tenant);
        c.register("envoy-1");
        c.record("envoy-1", probe(false, 500, 100)).unwrap();
        let s = c.status("envoy-1").unwrap();
        assert_eq!(s.state, ProxyState::Degraded);
        assert_eq!(s.consecutive_failures, 1);
    }

    #[test]
    fn threshold_consecutive_failures_moves_state_to_down() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Record.Down", "tenant-ph-rdn");
        let mut c = checker(tenant);
        c.register("envoy-1");
        for i in 0..3u64 {
            c.record("envoy-1", probe(false, 500, i)).unwrap();
        }
        assert_eq!(c.status("envoy-1").unwrap().state, ProxyState::Down);
    }

    #[test]
    fn success_after_failure_resets_to_live_and_clears_failures() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Record.RecoverLive", "tenant-ph-rrl");
        let mut c = checker(tenant);
        c.register("envoy-1");
        c.record("envoy-1", probe(false, 500, 100)).unwrap();
        c.record("envoy-1", probe(false, 500, 200)).unwrap();
        c.record("envoy-1", probe(true, 200, 300)).unwrap();
        let s = c.status("envoy-1").unwrap();
        assert_eq!(s.state, ProxyState::Live);
        assert_eq!(s.consecutive_failures, 0);
    }

    #[test]
    fn record_unknown_proxy_returns_not_registered() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Record.NotRegistered", "tenant-ph-rnr");
        let mut c = checker(tenant);
        let err = c.record("ghost", probe(true, 200, 0)).unwrap_err();
        assert!(matches!(err, HealthError::NotRegistered(_)));
    }

    #[test]
    fn record_failure_increments_total_and_records_timestamp() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Record.Failure.Counter", "tenant-ph-rfc");
        let mut c = checker(tenant);
        c.register("envoy-1");
        c.record("envoy-1", probe(false, 500, 100)).unwrap();
        let s = c.status("envoy-1").unwrap();
        assert_eq!(s.probes_total, 1);
        assert_eq!(s.probes_success, 0);
        assert_eq!(s.last_failure_ns, 100);
    }

    // ── Counts ────────────────────────────────────────────────────────────

    #[test]
    fn count_tracks_register() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Count", "tenant-ph-c");
        let mut c = checker(tenant);
        for i in 0..5 {
            c.register(format!("envoy-{i}"));
        }
        assert_eq!(c.count(), 5);
    }

    #[test]
    fn live_count_only_counts_live_proxies() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "LiveCount", "tenant-ph-lc");
        let mut c = checker(tenant);
        for i in 0..3 {
            c.register(format!("envoy-{i}"));
            c.record(&format!("envoy-{i}"), probe(true, 200, 0)).unwrap();
        }
        c.register("dead");
        for ts in 0..3u64 {
            c.record("dead", probe(false, 500, ts)).unwrap();
        }
        assert_eq!(c.live_count(), 3);
    }

    // ── Re-register idempotent ─────────────────────────────────────────────

    #[test]
    fn re_register_does_not_reset_state() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Register.Idempotent", "tenant-ph-rri");
        let mut c = checker(tenant);
        c.register("envoy-1");
        c.record("envoy-1", probe(false, 500, 100)).unwrap();
        c.register("envoy-1"); // no-op
        let s = c.status("envoy-1").unwrap();
        assert_eq!(s.consecutive_failures, 1);
    }

    // ── Threshold customisation ────────────────────────────────────────────

    #[test]
    fn custom_threshold_controls_down_transition() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "DownThreshold", "tenant-ph-dt");
        let mut c = ProxyHealthChecker::new(tenant, 5);
        c.register("envoy-1");
        for i in 0..4u64 {
            c.record("envoy-1", probe(false, 500, i)).unwrap();
        }
        assert_eq!(c.status("envoy-1").unwrap().state, ProxyState::Degraded);
        c.record("envoy-1", probe(false, 500, 4)).unwrap();
        assert_eq!(c.status("envoy-1").unwrap().state, ProxyState::Down);
    }

    // ── Status lookup ──────────────────────────────────────────────────────

    #[test]
    fn status_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Status.NotFound", "tenant-ph-snf");
        let c = checker(tenant);
        assert!(c.status("ghost").is_none());
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn proxy_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "State.Serde", "tenant-ph-sserde");
        for s in [ProxyState::Live, ProxyState::Degraded, ProxyState::Down] {
            let j = serde_json::to_string(&s).unwrap();
            let back: ProxyState = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn proxy_probe_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Probe.Serde", "tenant-ph-pserde");
        let p = probe(true, 200, 100);
        let s = serde_json::to_string(&p).unwrap();
        let back: ProxyProbe = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn proxy_status_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Status.Serde", "tenant-ph-stserde");
        let mut c = checker(tenant);
        c.register("envoy-1");
        c.record("envoy-1", probe(true, 200, 100)).unwrap();
        let s = c.status("envoy-1").unwrap().clone();
        let json = serde_json::to_string(&s).unwrap();
        let back: ProxyStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ── Mixed sequence ─────────────────────────────────────────────────────

    #[test]
    fn alternating_success_failure_does_not_reach_down() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/healthcheck.go", "Alternating", "tenant-ph-alt");
        let mut c = checker(tenant);
        c.register("envoy-1");
        for i in 0..6u64 {
            c.record("envoy-1", probe(i % 2 == 0, 200, i)).unwrap();
        }
        assert_ne!(c.status("envoy-1").unwrap().state, ProxyState::Down);
    }
}

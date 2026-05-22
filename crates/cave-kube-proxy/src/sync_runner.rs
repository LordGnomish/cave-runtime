// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Proxier sync loop driver — debounce + full-sync coordinator.
//!
//! Cite: `pkg/proxy/iptables/proxier.go:546` (syncRunner),
//! `:638` (syncProxyRules), `pkg/proxy/util/async/bounded_frequency_runner.go:32`
//! (BoundedFrequencyRunner — the min/max debounce primitive),
//! `cmd/kube-proxy/app/server.go:766` (Run loop).
//!
//! cave's `ProxySyncRunner` is the platform-agnostic driver; the real
//! iptables-restore / `nft -f` call lives behind a `SyncBackend` trait so
//! tests can capture the rendered payload without touching netfilter.

use crate::endpoints::EndpointSliceMap;
use crate::error::{KubeProxyError, KubeProxyResult};
use crate::iptables::IptablesProxier;
use crate::nftables::NftablesProxier;
use crate::proxy_config::{ProxyConfig, ProxyMode};
use crate::service::{ServiceChangeTracker, ServicePortInfo};
use std::time::{Duration, Instant};

/// Cite: `pkg/proxy/util/async/bounded_frequency_runner.go:32` —
/// debounce window: at most one sync per `min_interval`, at least one
/// sync per `max_interval`.
#[derive(Debug, Clone)]
pub struct BoundedFrequencyRunner {
    pub min_interval: Duration,
    pub max_interval: Duration,
    last_sync: Option<Instant>,
    dirty_since: Option<Instant>,
}

impl BoundedFrequencyRunner {
    pub fn new(min_interval: Duration, max_interval: Duration) -> Self {
        Self {
            min_interval,
            max_interval,
            last_sync: None,
            dirty_since: None,
        }
    }

    /// Mark the underlying state dirty — the next `should_sync` call will
    /// return true once `min_interval` has elapsed.
    pub fn mark_dirty(&mut self, now: Instant) {
        if self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
    }

    /// Returns true if a sync should fire now. The decision rule is:
    ///
    /// * Never run faster than `min_interval` (debounce).
    /// * Never run slower than `max_interval`, even when clean (keep-alive).
    pub fn should_sync(&self, now: Instant) -> bool {
        if let Some(last) = self.last_sync {
            if now.duration_since(last) < self.min_interval {
                return false;
            }
            if self.dirty_since.is_some() {
                return true;
            }
            now.duration_since(last) >= self.max_interval
        } else {
            true
        }
    }

    /// Record that a sync just completed at `now`.
    pub fn mark_synced(&mut self, now: Instant) {
        self.last_sync = Some(now);
        self.dirty_since = None;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }
}

/// Cite: `pkg/proxy/iptables/proxier.go:546` (syncRunner field) — the
/// proxier owns a sync runner that the consumer drives once per tick.
#[derive(Debug)]
pub struct ProxySyncRunner {
    pub tenant_id: String,
    pub config: ProxyConfig,
    pub change_tracker: ServiceChangeTracker,
    pub endpoint_cache: EndpointSliceMap,
    pub iptables: IptablesProxier,
    pub nftables: NftablesProxier,
    pub debounce: BoundedFrequencyRunner,
    sync_count: u64,
}

impl ProxySyncRunner {
    pub fn new(tenant_id: impl Into<String> + Clone, config: ProxyConfig) -> KubeProxyResult<Self> {
        config.validate()?;
        let tid = tenant_id.clone().into();
        let min = Duration::from_secs(config.min_sync_period_secs.max(1) as u64);
        let max = Duration::from_secs(config.sync_period_secs as u64);
        Ok(Self {
            tenant_id: tid.clone(),
            config,
            change_tracker: ServiceChangeTracker::new(tid.clone()),
            endpoint_cache: EndpointSliceMap::new(tid.clone()),
            iptables: IptablesProxier::new(tid.clone()),
            nftables: NftablesProxier::new(tid),
            debounce: BoundedFrequencyRunner::new(min, max),
            sync_count: 0,
        })
    }

    /// Cite: `pkg/proxy/iptables/proxier.go:638` (syncProxyRules) —
    /// full datapath emission. Returns the rendered ruleset payload the
    /// caller hands to iptables-restore / `nft -f`.
    pub fn sync_proxy_rules(&mut self, services: &[ServicePortInfo]) -> KubeProxyResult<Vec<String>> {
        // Cross-tenant guard: every input must match the runner's tenant.
        for s in services {
            if s.tenant_id != self.tenant_id {
                return Err(KubeProxyError::CrossTenantDenied {
                    store: self.tenant_id.clone(),
                    req: s.tenant_id.clone(),
                });
            }
        }

        let mut payload = Vec::new();
        match self.config.mode {
            ProxyMode::Iptables => {
                payload.extend(self.iptables.build_kube_services_rules(services)?);
                payload.extend(self.iptables.build_kube_nodeports_rules(services)?);
                payload.push(self.iptables.build_kube_services_nodeports_terminator());
            }
            ProxyMode::Nftables => {
                payload.extend(self.nftables.build_table_scaffold());
                let svc_ips = self.nftables.build_service_ips_map_entries(services)?;
                if !svc_ips.is_empty() {
                    payload.push(format!("add element inet kube-proxy service-ips {{"));
                    payload.extend(svc_ips);
                    payload.push("}".to_string());
                }
                let np = self.nftables.build_service_nodeports_map_entries(services)?;
                if !np.is_empty() {
                    payload.push(format!("add element inet kube-proxy service-nodeports {{"));
                    payload.extend(np);
                    payload.push("}".to_string());
                }
                payload.extend(self.nftables.build_jump_rules());
            }
            ProxyMode::Ipvs => {
                // Sync into the eBPF-IPVS shim is handled in cave-net;
                // at this layer we emit the same iptables backstop rules
                // so existing iptables tooling still sees a coherent view.
                payload.extend(self.iptables.build_kube_services_rules(services)?);
            }
        }

        self.sync_count += 1;
        self.debounce.mark_synced(Instant::now());
        Ok(payload)
    }

    pub fn sync_count(&self) -> u64 {
        self.sync_count
    }

    /// Cite: `pkg/proxy/iptables/proxier.go:546` (syncRunner.Run loop) —
    /// the proxier signals dirtiness when it absorbs a Service or
    /// EndpointSlice event.
    pub fn on_event(&mut self) {
        self.debounce.mark_dirty(Instant::now());
    }

    /// Cite: `pkg/proxy/iptables/proxier.go:679` (probeTunneling) —
    /// returns true when the proxier currently has at least one Service
    /// programmed; used by the readiness probe.
    pub fn is_initialized(&self) -> bool {
        self.sync_count > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Protocol, ServicePortName};
    use std::net::IpAddr;

    fn svc_for(t: &str, name: &str, ip: &str, port: u16) -> ServicePortInfo {
        ServicePortInfo::cluster_ip_only(
            t,
            ServicePortName::new("default", name, "http"),
            IpAddr::V4(ip.parse().unwrap()),
            port,
            Protocol::Tcp,
        )
    }

    #[test]
    fn debounce_first_call_runs() {
        let r = BoundedFrequencyRunner::new(Duration::from_secs(1), Duration::from_secs(30));
        assert!(r.should_sync(Instant::now()));
    }

    #[test]
    fn debounce_blocks_below_min_interval() {
        let mut r = BoundedFrequencyRunner::new(Duration::from_secs(1), Duration::from_secs(30));
        let t0 = Instant::now();
        r.mark_synced(t0);
        r.mark_dirty(t0);
        assert!(!r.should_sync(t0));
    }

    #[test]
    fn debounce_keepalive_fires_after_max_interval() {
        let r_max = Duration::from_millis(10);
        let mut r = BoundedFrequencyRunner::new(Duration::from_millis(1), r_max);
        let t0 = Instant::now();
        r.mark_synced(t0);
        std::thread::sleep(Duration::from_millis(20));
        assert!(r.should_sync(Instant::now()));
    }

    #[test]
    fn runner_builds_iptables_payload() {
        let mut cfg = ProxyConfig::default();
        cfg.mode = ProxyMode::Iptables;
        let mut runner = ProxySyncRunner::new("t1", cfg).unwrap();
        let svcs = vec![svc_for("t1", "web", "10.0.0.1", 80)];
        let payload = runner.sync_proxy_rules(&svcs).unwrap();
        assert!(payload.iter().any(|l| l.contains("KUBE-SERVICES")));
        assert_eq!(runner.sync_count(), 1);
    }

    #[test]
    fn runner_builds_nftables_payload() {
        let mut cfg = ProxyConfig::default();
        cfg.mode = ProxyMode::Nftables;
        let mut runner = ProxySyncRunner::new("t1", cfg).unwrap();
        let svcs = vec![svc_for("t1", "web", "10.0.0.1", 80)];
        let payload = runner.sync_proxy_rules(&svcs).unwrap();
        assert!(payload.iter().any(|l| l.contains("table inet kube-proxy")));
        assert!(payload.iter().any(|l| l.contains("service-ips")));
    }

    #[test]
    fn runner_rejects_cross_tenant_inputs() {
        let cfg = ProxyConfig::default();
        let mut runner = ProxySyncRunner::new("t1", cfg).unwrap();
        let svcs = vec![svc_for("t-other", "web", "10.0.0.1", 80)];
        assert!(matches!(
            runner.sync_proxy_rules(&svcs),
            Err(KubeProxyError::CrossTenantDenied { .. })
        ));
    }

    #[test]
    fn runner_on_event_marks_dirty() {
        let cfg = ProxyConfig::default();
        let mut runner = ProxySyncRunner::new("t1", cfg).unwrap();
        assert!(!runner.debounce.is_dirty());
        runner.on_event();
        assert!(runner.debounce.is_dirty());
    }

    #[test]
    fn runner_is_initialized_after_first_sync() {
        let cfg = ProxyConfig::default();
        let mut runner = ProxySyncRunner::new("t1", cfg).unwrap();
        assert!(!runner.is_initialized());
        runner.sync_proxy_rules(&[]).unwrap();
        assert!(runner.is_initialized());
    }
}

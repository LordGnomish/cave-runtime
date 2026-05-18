// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cilium agent Status API — `cilium status` shape.
//!
//! Mirrors `pkg/status/status.go` plus the
//! `api/v1/models/StatusResponse` shape consumed by the `cilium status`
//! CLI. Each subsystem reports a `ComponentStatus` (Ok/Degraded/Failure
//! + message). The aggregator computes the daemon-wide state.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ComponentName {
    Kvstore,
    Kubernetes,
    Cilium,
    NodeMonitor,
    Cluster,
    ContainerRuntime,
    IPAM,
    Encryption,
    BandwidthManager,
    Hubble,
    Authentication,
    Bgp,
    L2Announcer,
    Proxy,
}

impl std::fmt::Display for ComponentName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl ComponentName {
    pub fn label(self) -> &'static str {
        match self {
            ComponentName::Kvstore => "kvstore",
            ComponentName::Kubernetes => "kubernetes",
            ComponentName::Cilium => "cilium",
            ComponentName::NodeMonitor => "node-monitor",
            ComponentName::Cluster => "cluster",
            ComponentName::ContainerRuntime => "container-runtime",
            ComponentName::IPAM => "ipam",
            ComponentName::Encryption => "encryption",
            ComponentName::BandwidthManager => "bandwidth-manager",
            ComponentName::Hubble => "hubble",
            ComponentName::Authentication => "authentication",
            ComponentName::Bgp => "bgp-control-plane",
            ComponentName::L2Announcer => "l2-announcer",
            ComponentName::Proxy => "proxy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DaemonState {
    Ok,
    Degraded,
    Failure,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub state: DaemonState,
    pub message: String,
    pub last_updated_ns: u64,
}

impl ComponentStatus {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self { state: DaemonState::Ok, message: msg.into(), last_updated_ns: 0 }
    }
    pub fn disabled() -> Self {
        Self { state: DaemonState::Disabled, message: "disabled".into(), last_updated_ns: 0 }
    }
    pub fn degraded(msg: impl Into<String>) -> Self {
        Self { state: DaemonState::Degraded, message: msg.into(), last_updated_ns: 0 }
    }
    pub fn failure(msg: impl Into<String>) -> Self {
        Self { state: DaemonState::Failure, message: msg.into(), last_updated_ns: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub overall: DaemonState,
    pub components: BTreeMap<String, ComponentStatus>,
    pub stale_since_ns: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StatusError {
    #[error("component {0} not registered")]
    ComponentNotFound(ComponentName),
    #[error("tenant {tenant} cannot mutate status board owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct StatusBoard {
    pub tenant: TenantId,
    /// Stale threshold — component status older than this triggers Degraded.
    pub stale_threshold_ns: u64,
    components: BTreeMap<ComponentName, ComponentStatus>,
}

impl StatusBoard {
    pub fn new(tenant: TenantId, stale_seconds: u64) -> Self {
        Self {
            tenant,
            stale_threshold_ns: stale_seconds * 1_000_000_000,
            components: BTreeMap::new(),
        }
    }

    pub fn report(&mut self, component: ComponentName, mut status: ComponentStatus, now_ns: u64) {
        status.last_updated_ns = now_ns;
        self.components.insert(component, status);
    }

    pub fn unregister(&mut self, component: ComponentName) -> Result<(), StatusError> {
        self.components.remove(&component).ok_or(StatusError::ComponentNotFound(component))?;
        Ok(())
    }

    pub fn lookup(&self, component: ComponentName) -> Option<&ComponentStatus> {
        self.components.get(&component)
    }

    pub fn count(&self) -> usize {
        self.components.len()
    }

    /// Aggregate the daemon-wide state. Mirrors
    /// `pkg/status/status.go::Collect`. Disabled components don't
    /// influence the overall verdict; otherwise the worst (Failure >
    /// Degraded > Ok) wins. Stale components are forced to Degraded.
    pub fn aggregate(&self, now_ns: u64) -> DaemonStatus {
        let mut overall = DaemonState::Ok;
        let mut out: BTreeMap<String, ComponentStatus> = BTreeMap::new();
        let mut stale_seen = u64::MAX;
        for (name, status) in &self.components {
            let mut effective = status.clone();
            if !matches!(effective.state, DaemonState::Disabled) {
                let elapsed = now_ns.saturating_sub(status.last_updated_ns);
                if elapsed >= self.stale_threshold_ns {
                    effective.state = DaemonState::Degraded;
                    if effective.message.is_empty() {
                        effective.message = "stale".into();
                    } else {
                        effective.message = format!("stale: {}", effective.message);
                    }
                    if status.last_updated_ns < stale_seen {
                        stale_seen = status.last_updated_ns;
                    }
                }
            }
            // Worst-state aggregation.
            let worst_so_far = match (overall, effective.state) {
                (_, DaemonState::Disabled) => overall,
                (DaemonState::Failure, _) => DaemonState::Failure,
                (_, DaemonState::Failure) => DaemonState::Failure,
                (DaemonState::Degraded, _) => DaemonState::Degraded,
                (_, DaemonState::Degraded) => DaemonState::Degraded,
                (a, _) => a,
            };
            overall = worst_so_far;
            out.insert(name.label().to_string(), effective);
        }
        DaemonStatus {
            overall,
            components: out,
            stale_since_ns: if stale_seen == u64::MAX { 0 } else { stale_seen },
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/status/status.go", "Status");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn board(tenant: TenantId) -> StatusBoard {
        StatusBoard::new(tenant, 30)
    }

    // ── Component label mapping ─────────────────────────────────────────────

    #[test]
    fn component_labels_match_cli_output() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "ComponentName.Label", "tenant-st-lbl");
        assert_eq!(ComponentName::Kvstore.label(), "kvstore");
        assert_eq!(ComponentName::Kubernetes.label(), "kubernetes");
        assert_eq!(ComponentName::IPAM.label(), "ipam");
        assert_eq!(ComponentName::Hubble.label(), "hubble");
        assert_eq!(ComponentName::Bgp.label(), "bgp-control-plane");
        assert_eq!(ComponentName::L2Announcer.label(), "l2-announcer");
        assert_eq!(ComponentName::Proxy.label(), "proxy");
    }

    // ── ComponentStatus constructors ────────────────────────────────────────

    #[test]
    fn component_status_ok_constructor() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "Status.Ok", "tenant-st-ok");
        let s = ComponentStatus::ok("running");
        assert_eq!(s.state, DaemonState::Ok);
        assert_eq!(s.message, "running");
    }

    #[test]
    fn component_status_disabled_uses_default_message() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "Status.Disabled", "tenant-st-dis");
        let s = ComponentStatus::disabled();
        assert_eq!(s.state, DaemonState::Disabled);
        assert_eq!(s.message, "disabled");
    }

    #[test]
    fn component_status_degraded_carries_reason() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "Status.Degraded", "tenant-st-deg");
        let s = ComponentStatus::degraded("kvstore connection flaky");
        assert_eq!(s.state, DaemonState::Degraded);
        assert!(s.message.contains("flaky"));
    }

    #[test]
    fn component_status_failure_carries_reason() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "Status.Failure", "tenant-st-fail");
        let s = ComponentStatus::failure("BPF program failed to load");
        assert_eq!(s.state, DaemonState::Failure);
    }

    // ── Reporting ──────────────────────────────────────────────────────────

    #[test]
    fn report_records_status_with_timestamp() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Report", "tenant-st-rep");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("connected"), 100);
        let s = b.lookup(ComponentName::Kvstore).unwrap();
        assert_eq!(s.last_updated_ns, 100);
        assert_eq!(s.state, DaemonState::Ok);
    }

    #[test]
    fn report_overwrites_existing() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Report.Overwrite", "tenant-st-repov");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("connected"), 100);
        b.report(ComponentName::Kvstore, ComponentStatus::failure("disconnected"), 200);
        let s = b.lookup(ComponentName::Kvstore).unwrap();
        assert_eq!(s.state, DaemonState::Failure);
    }

    #[test]
    fn unregister_drops_component() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Unregister", "tenant-st-unr");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 100);
        b.unregister(ComponentName::Kvstore).unwrap();
        assert!(b.lookup(ComponentName::Kvstore).is_none());
    }

    #[test]
    fn unregister_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Unregister.NotFound", "tenant-st-unrnf");
        let mut b = board(tenant);
        let err = b.unregister(ComponentName::Kvstore).unwrap_err();
        assert!(matches!(err, StatusError::ComponentNotFound(_)));
    }

    #[test]
    fn count_tracks_reports() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Count", "tenant-st-cnt");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 0);
        b.report(ComponentName::Kubernetes, ComponentStatus::ok("ok"), 0);
        assert_eq!(b.count(), 2);
    }

    // ── Aggregate ──────────────────────────────────────────────────────────

    #[test]
    fn aggregate_all_ok_returns_ok() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.AllOk", "tenant-st-agok");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("connected"), 100);
        b.report(ComponentName::IPAM, ComponentStatus::ok("ready"), 100);
        let agg = b.aggregate(100);
        assert_eq!(agg.overall, DaemonState::Ok);
    }

    #[test]
    fn aggregate_with_one_degraded_returns_degraded() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.Degraded", "tenant-st-agdeg");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 100);
        b.report(ComponentName::IPAM, ComponentStatus::degraded("low pool"), 100);
        let agg = b.aggregate(100);
        assert_eq!(agg.overall, DaemonState::Degraded);
    }

    #[test]
    fn aggregate_with_one_failure_returns_failure() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.Failure", "tenant-st-agfail");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::degraded("flaky"), 100);
        b.report(ComponentName::IPAM, ComponentStatus::failure("exhausted"), 100);
        let agg = b.aggregate(100);
        assert_eq!(agg.overall, DaemonState::Failure);
    }

    #[test]
    fn aggregate_disabled_component_does_not_influence_overall() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.SkipDisabled", "tenant-st-agdis");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 100);
        b.report(ComponentName::Encryption, ComponentStatus::disabled(), 100);
        let agg = b.aggregate(100);
        assert_eq!(agg.overall, DaemonState::Ok);
    }

    #[test]
    fn aggregate_empty_returns_ok() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.Empty", "tenant-st-agem");
        let b = board(tenant);
        let agg = b.aggregate(100);
        assert_eq!(agg.overall, DaemonState::Ok);
    }

    // ── Stale handling ─────────────────────────────────────────────────────

    #[test]
    fn aggregate_stale_component_marked_degraded() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.Stale", "tenant-st-stale");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 0);
        // 60s elapsed > 30s threshold.
        let agg = b.aggregate(60_000_000_000);
        assert_eq!(agg.overall, DaemonState::Degraded);
        assert!(agg.components.get("kvstore").unwrap().message.starts_with("stale"));
    }

    #[test]
    fn aggregate_recent_component_not_marked_stale() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.Fresh", "tenant-st-fresh");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 0);
        let agg = b.aggregate(10_000_000_000);
        assert_eq!(agg.overall, DaemonState::Ok);
    }

    #[test]
    fn aggregate_disabled_not_marked_stale_even_when_old() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.DisabledNotStale", "tenant-st-disns");
        let mut b = board(tenant);
        b.report(ComponentName::Encryption, ComponentStatus::disabled(), 0);
        let agg = b.aggregate(60_000_000_000);
        assert_eq!(agg.overall, DaemonState::Ok);
        assert_eq!(agg.components.get("encryption").unwrap().state, DaemonState::Disabled);
    }

    #[test]
    fn aggregate_stale_since_ns_records_oldest_observed() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.StaleSince", "tenant-st-stsince");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 0);
        b.report(ComponentName::Kubernetes, ComponentStatus::ok("ok"), 5_000_000_000);
        let agg = b.aggregate(60_000_000_000);
        // Both stale; the oldest is at t=0.
        assert_eq!(agg.stale_since_ns, 0);
    }

    // ── Components map ─────────────────────────────────────────────────────

    #[test]
    fn aggregate_components_map_keyed_by_label() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.MapKeys", "tenant-st-mk");
        let mut b = board(tenant);
        b.report(ComponentName::Hubble, ComponentStatus::ok("ok"), 0);
        b.report(ComponentName::Bgp, ComponentStatus::ok("ok"), 0);
        let agg = b.aggregate(0);
        assert!(agg.components.contains_key("hubble"));
        assert!(agg.components.contains_key("bgp-control-plane"));
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn daemon_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "DaemonState.Serde", "tenant-st-dsserde");
        for s in [DaemonState::Ok, DaemonState::Degraded, DaemonState::Failure, DaemonState::Disabled] {
            let j = serde_json::to_string(&s).unwrap();
            let back: DaemonState = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn component_status_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "ComponentStatus.Serde", "tenant-st-csserde");
        let s = ComponentStatus::ok("connected");
        let j = serde_json::to_string(&s).unwrap();
        let back: ComponentStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn daemon_status_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "DaemonStatus.Serde", "tenant-st-dstserde");
        let mut b = board(tenant);
        b.report(ComponentName::Kvstore, ComponentStatus::ok("ok"), 0);
        let agg = b.aggregate(0);
        let j = serde_json::to_string(&agg).unwrap();
        let back: DaemonStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, agg);
    }

    // ── Edge: priority ordering ──────────────────────────────────────────────

    #[test]
    fn daemon_state_priority_failure_gt_degraded_gt_ok() {
        let (_c, _t) = cilium_test_ctx!("pkg/status/status.go", "DaemonState.Order", "tenant-st-ord");
        assert!(DaemonState::Failure > DaemonState::Degraded);
        assert!(DaemonState::Degraded > DaemonState::Ok);
    }

    #[test]
    fn aggregate_component_status_carries_through_to_output_map() {
        let (_c, tenant) = cilium_test_ctx!("pkg/status/status.go", "Aggregate.PassThrough", "tenant-st-pt");
        let mut b = board(tenant);
        b.report(ComponentName::IPAM, ComponentStatus::degraded("low pool"), 100);
        let agg = b.aggregate(100);
        assert_eq!(agg.components.get("ipam").unwrap().message, "low pool");
    }
}

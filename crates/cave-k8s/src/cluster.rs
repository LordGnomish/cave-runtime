// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `ControlPlane` — the umbrella facade.  Constructs and owns the
//! configured set of subsystems, exposes a uniform `health()` /
//! `phase()` / `components()` view, and drives the cluster bootstrap
//! sequence (etcd → apiserver → controller-manager → scheduler →
//! kube-proxy → kubelet → cri → cloud-controller-manager).

use crate::models::{ClusterPhase, ComponentHealth, ComponentName, NodeRole};
use crate::state::State;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterConfig {
    pub name: String,
    pub node_role: NodeRole,
    pub enable_cloud_provider: bool,
    pub enable_pqc_sa_tokens: bool,
    /// kube-proxy datapath: one of `iptables`, `nftables`, `ebpf`.
    pub proxy_mode: String,
    /// Pod CIDR (CIDR notation), e.g. `10.244.0.0/16`.
    pub pod_cidr: String,
    /// Service CIDR (CIDR notation), e.g. `10.96.0.0/12`.
    pub service_cidr: String,
    /// DNS domain — defaults to `cluster.local`.
    pub dns_domain: String,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            name: "cave-k8s".into(),
            node_role: NodeRole::Hybrid,
            enable_cloud_provider: false,
            enable_pqc_sa_tokens: true,
            proxy_mode: "nftables".into(),
            pod_cidr: "10.244.0.0/16".into(),
            service_cidr: "10.96.0.0/12".into(),
            dns_domain: "cluster.local".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterStatus {
    pub phase: ClusterPhase,
    pub components: BTreeMap<String, ComponentHealth>,
    pub healthy_components: u32,
    pub total_components: u32,
}

impl ClusterStatus {
    pub fn is_healthy(&self) -> bool {
        self.healthy_components == self.total_components && self.phase == ClusterPhase::Running
    }
}

#[derive(Clone)]
pub struct ControlPlane {
    pub config: ClusterConfig,
    pub state: Arc<State>,
    /// Per-component health snapshot — mutated by `start()` and
    /// inspected by `status()`.  Behind a `parking_lot::RwLock`-style
    /// `std::sync::RwLock` so that `Clone` remains cheap.
    health: Arc<std::sync::RwLock<BTreeMap<ComponentName, ComponentHealth>>>,
    phase: Arc<std::sync::RwLock<ClusterPhase>>,
}

impl ControlPlane {
    pub fn new(config: ClusterConfig) -> Self {
        Self {
            config,
            state: Arc::new(State::default()),
            health: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            phase: Arc::new(std::sync::RwLock::new(ClusterPhase::Pending)),
        }
    }

    pub fn with_state(mut self, state: Arc<State>) -> Self {
        self.state = state;
        self
    }

    pub fn phase(&self) -> ClusterPhase {
        *self.phase.read().expect("phase lock")
    }

    /// Drive the cluster from `Pending` through `Bootstrapping` to
    /// `Running`. Components are brought up in K8s-canonical order;
    /// `cloud-controller-manager` is skipped when `enable_cloud_provider`
    /// is false.
    pub fn start(&self) {
        *self.phase.write().expect("phase lock") = ClusterPhase::Bootstrapping;
        let order: &[ComponentName] = &[
            ComponentName::Etcd,
            ComponentName::Apiserver,
            ComponentName::ControllerManager,
            ComponentName::Scheduler,
            ComponentName::KubeProxy,
            ComponentName::Kubelet,
            ComponentName::Cri,
        ];
        {
            let mut h = self.health.write().expect("health lock");
            for c in order {
                h.insert(*c, ComponentHealth::Healthy);
            }
            if self.config.enable_cloud_provider {
                h.insert(ComponentName::CloudControllerManager, ComponentHealth::Healthy);
            }
        }
        *self.phase.write().expect("phase lock") = ClusterPhase::Running;
    }

    pub fn shutdown(&self) {
        *self.phase.write().expect("phase lock") = ClusterPhase::Draining;
        self.health.write().expect("health lock").clear();
    }

    pub fn mark_unhealthy(&self, c: ComponentName) {
        self.health
            .write()
            .expect("health lock")
            .insert(c, ComponentHealth::Unhealthy);
    }

    pub fn mark_degraded(&self, c: ComponentName) {
        self.health
            .write()
            .expect("health lock")
            .insert(c, ComponentHealth::Degraded);
    }

    pub fn status(&self) -> ClusterStatus {
        let h = self.health.read().expect("health lock");
        let phase = *self.phase.read().expect("phase lock");
        let total_components = h.len() as u32;
        let healthy_components = h
            .values()
            .filter(|v| matches!(v, ComponentHealth::Healthy))
            .count() as u32;
        let mut components = BTreeMap::new();
        for (k, v) in h.iter() {
            components.insert(k.as_str().to_string(), *v);
        }
        ClusterStatus {
            phase,
            components,
            healthy_components,
            total_components,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_match_charter() {
        let c = ClusterConfig::default();
        assert_eq!(c.proxy_mode, "nftables");
        assert_eq!(c.dns_domain, "cluster.local");
        assert!(c.enable_pqc_sa_tokens, "PQC SA tokens default-on per Charter v2");
        assert!(!c.enable_cloud_provider);
    }

    #[test]
    fn bootstrap_runs_seven_components_without_cloud() {
        let cp = ControlPlane::new(ClusterConfig::default());
        assert_eq!(cp.phase(), ClusterPhase::Pending);
        cp.start();
        assert_eq!(cp.phase(), ClusterPhase::Running);
        let s = cp.status();
        assert_eq!(s.total_components, 7);
        assert_eq!(s.healthy_components, 7);
        assert!(s.is_healthy());
    }

    #[test]
    fn cloud_provider_flips_to_eight_components() {
        let mut cfg = ClusterConfig::default();
        cfg.enable_cloud_provider = true;
        let cp = ControlPlane::new(cfg);
        cp.start();
        let s = cp.status();
        assert_eq!(s.total_components, 8);
        assert!(s.components.contains_key("cloud-controller-manager"));
    }

    #[test]
    fn mark_unhealthy_degrades_status() {
        let cp = ControlPlane::new(ClusterConfig::default());
        cp.start();
        cp.mark_unhealthy(ComponentName::Scheduler);
        let s = cp.status();
        assert!(!s.is_healthy());
        assert_eq!(s.components.get("scheduler"), Some(&ComponentHealth::Unhealthy));
    }

    #[test]
    fn shutdown_clears_components_and_sets_draining() {
        let cp = ControlPlane::new(ClusterConfig::default());
        cp.start();
        cp.shutdown();
        assert_eq!(cp.phase(), ClusterPhase::Draining);
        assert_eq!(cp.status().total_components, 0);
    }

    #[test]
    fn status_roundtrips_json() {
        let cp = ControlPlane::new(ClusterConfig::default());
        cp.start();
        let s = cp.status();
        let json = serde_json::to_string(&s).unwrap();
        let back: ClusterStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}

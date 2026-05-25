// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! VM-mesh expansion — `pilot/pkg/networking/serviceentry/external/`.
//!
//! Istio's "VM mesh" feature lets non-Kubernetes workloads (bare-metal
//! and VM-resident services) participate in the mesh by enrolling them
//! as `WorkloadEntry` resources under a parent `ServiceEntry`. This
//! module implements the enrolment manager: lifecycle of VM workloads,
//! health bookkeeping, mesh-membership probes, and the SPIFFE identity
//! issuance hook that ambient ztunnel uses to mTLS-tunnel VM traffic.
//!
//! Ambient-mode quirk: VM workloads bypass the Envoy sidecar entirely;
//! ztunnel acts as the L4 zero-trust tunnel front-end. Each registered
//! VM workload gets a SPIFFE ID derived from its service account +
//! network identity.

use crate::models::{ServiceEntry, WorkloadEntry};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

// SE in this codebase carries a non-optional `namespace: String`; WorkloadEntry
// carries an `Option<String>`. The enrolment path falls back to the parent SE's
// namespace when the workload's own is unset.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmHealth {
    Healthy,
    Unhealthy,
    Pending,
}

#[derive(Debug, Clone)]
pub struct VmWorkloadState {
    pub workload: WorkloadEntry,
    pub parent_service_entry: String,
    pub health: VmHealth,
    pub last_probe: Option<Instant>,
    pub spiffe_id: String,
}

impl VmWorkloadState {
    pub fn new(workload: WorkloadEntry, parent_se: &str, trust_domain: &str) -> Self {
        let sa = workload
            .service_account
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let ns = workload
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let spiffe_id = format!("spiffe://{}/ns/{}/sa/{}", trust_domain, ns, sa);
        Self {
            workload,
            parent_service_entry: parent_se.to_string(),
            health: VmHealth::Pending,
            last_probe: None,
            spiffe_id,
        }
    }
}

/// VM-mesh enrolment manager.
pub struct VmMesh {
    pub trust_domain: String,
    pub probe_interval: Duration,
    workloads: RwLock<HashMap<String, VmWorkloadState>>,
}

impl VmMesh {
    pub fn new(trust_domain: impl Into<String>) -> Self {
        Self {
            trust_domain: trust_domain.into(),
            probe_interval: Duration::from_secs(15),
            workloads: RwLock::new(HashMap::new()),
        }
    }

    /// Enrol a VM under its parent ServiceEntry. Returns the assigned SPIFFE ID.
    pub fn enrol(&self, parent: &ServiceEntry, workload: WorkloadEntry) -> String {
        let state = VmWorkloadState::new(workload, &parent.name, &self.trust_domain);
        let key = format!("{}/{}", parent.name, state.workload.address);
        let spiffe_id = state.spiffe_id.clone();
        self.workloads.write().unwrap().insert(key, state);
        spiffe_id
    }

    pub fn deregister(&self, parent: &ServiceEntry, address: &str) -> bool {
        let key = format!("{}/{}", parent.name, address);
        self.workloads.write().unwrap().remove(&key).is_some()
    }

    /// Record a probe outcome — used by the periodic health-check loop.
    pub fn record_probe(&self, parent: &str, address: &str, healthy: bool) -> bool {
        let key = format!("{}/{}", parent, address);
        let mut guard = self.workloads.write().unwrap();
        let Some(state) = guard.get_mut(&key) else {
            return false;
        };
        state.last_probe = Some(Instant::now());
        state.health = if healthy {
            VmHealth::Healthy
        } else {
            VmHealth::Unhealthy
        };
        true
    }

    pub fn health(&self, parent: &str, address: &str) -> Option<VmHealth> {
        let key = format!("{}/{}", parent, address);
        self.workloads.read().unwrap().get(&key).map(|s| s.health)
    }

    /// Return all VM workloads for the given parent ServiceEntry — feeds the
    /// xDS / ambient endpoint slice.
    pub fn by_parent(&self, parent: &str) -> Vec<VmWorkloadState> {
        self.workloads
            .read()
            .unwrap()
            .values()
            .filter(|s| s.parent_service_entry == parent)
            .cloned()
            .collect()
    }

    /// Healthy endpoints only — ambient ztunnel reads this slice.
    pub fn healthy_endpoints(&self, parent: &str) -> Vec<WorkloadEntry> {
        self.workloads
            .read()
            .unwrap()
            .values()
            .filter(|s| s.parent_service_entry == parent && s.health == VmHealth::Healthy)
            .map(|s| s.workload.clone())
            .collect()
    }

    pub fn count(&self) -> usize {
        self.workloads.read().unwrap().len()
    }

    pub fn spiffe_id_for(&self, parent: &str, address: &str) -> Option<String> {
        let key = format!("{}/{}", parent, address);
        self.workloads
            .read()
            .unwrap()
            .get(&key)
            .map(|s| s.spiffe_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ServiceEntry, ServiceLocation, ServicePort, ServiceResolution};
    use chrono::Utc;
    use std::collections::HashMap;

    fn mk_service_entry(name: &str) -> ServiceEntry {
        ServiceEntry {
            name: name.into(),
            namespace: "default".into(),
            hosts: vec!["vm.example.com".into()],
            addresses: vec![],
            ports: vec![ServicePort {
                name: "http".into(),
                number: 80,
                protocol: "HTTP".into(),
                target_port: None,
            }],
            location: ServiceLocation::MeshInternal,
            resolution: ServiceResolution::Static,
            endpoints: vec![],
            export_to: vec![],
            subject_alt_names: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn mk_workload(addr: &str, sa: Option<&str>, ns: Option<&str>) -> WorkloadEntry {
        let mut w = WorkloadEntry::new(addr);
        w.service_account = sa.map(String::from);
        w.namespace = ns.map(String::from);
        w
    }

    #[test]
    fn enrol_assigns_spiffe_id_from_service_account() {
        let mesh = VmMesh::new("cluster.local");
        let se = mk_service_entry("vm-svc");
        let id = mesh.enrol(&se, mk_workload("10.0.0.1", Some("api-vm"), Some("prod")));
        assert_eq!(id, "spiffe://cluster.local/ns/prod/sa/api-vm");
        assert_eq!(mesh.count(), 1);
    }

    #[test]
    fn enrol_defaults_namespace_and_service_account() {
        let mesh = VmMesh::new("cluster.local");
        let se = mk_service_entry("vm-svc");
        let id = mesh.enrol(&se, mk_workload("10.0.0.1", None, None));
        assert_eq!(id, "spiffe://cluster.local/ns/default/sa/default");
    }

    #[test]
    fn deregister_removes_state() {
        let mesh = VmMesh::new("cluster.local");
        let se = mk_service_entry("vm-svc");
        mesh.enrol(&se, mk_workload("10.0.0.1", None, None));
        assert!(mesh.deregister(&se, "10.0.0.1"));
        assert_eq!(mesh.count(), 0);
        assert!(!mesh.deregister(&se, "10.0.0.99"));
    }

    #[test]
    fn record_probe_flips_health_and_records_time() {
        let mesh = VmMesh::new("cluster.local");
        let se = mk_service_entry("vm-svc");
        mesh.enrol(&se, mk_workload("10.0.0.1", None, None));
        assert_eq!(mesh.health("vm-svc", "10.0.0.1"), Some(VmHealth::Pending));
        mesh.record_probe("vm-svc", "10.0.0.1", true);
        assert_eq!(mesh.health("vm-svc", "10.0.0.1"), Some(VmHealth::Healthy));
        mesh.record_probe("vm-svc", "10.0.0.1", false);
        assert_eq!(mesh.health("vm-svc", "10.0.0.1"), Some(VmHealth::Unhealthy));
    }

    #[test]
    fn healthy_endpoints_filters_unhealthy() {
        let mesh = VmMesh::new("cluster.local");
        let se = mk_service_entry("vm-svc");
        mesh.enrol(&se, mk_workload("10.0.0.1", None, None));
        mesh.enrol(&se, mk_workload("10.0.0.2", None, None));
        mesh.enrol(&se, mk_workload("10.0.0.3", None, None));
        mesh.record_probe("vm-svc", "10.0.0.1", true);
        mesh.record_probe("vm-svc", "10.0.0.2", false);
        mesh.record_probe("vm-svc", "10.0.0.3", true);
        let healthy = mesh.healthy_endpoints("vm-svc");
        assert_eq!(healthy.len(), 2);
    }

    #[test]
    fn record_probe_on_unknown_returns_false() {
        let mesh = VmMesh::new("cluster.local");
        assert!(!mesh.record_probe("nope", "1.1.1.1", true));
    }

    #[test]
    fn by_parent_partitions_correctly() {
        let mesh = VmMesh::new("cluster.local");
        let a = mk_service_entry("a-svc");
        let b = mk_service_entry("b-svc");
        mesh.enrol(&a, mk_workload("10.0.0.1", None, None));
        mesh.enrol(&a, mk_workload("10.0.0.2", None, None));
        mesh.enrol(&b, mk_workload("10.0.0.3", None, None));
        assert_eq!(mesh.by_parent("a-svc").len(), 2);
        assert_eq!(mesh.by_parent("b-svc").len(), 1);
        assert_eq!(mesh.by_parent("missing").len(), 0);
    }

    #[test]
    fn spiffe_id_lookup_after_enrol() {
        let mesh = VmMesh::new("trust.org");
        let se = mk_service_entry("svc");
        mesh.enrol(&se, mk_workload("1.2.3.4", Some("alice"), Some("team-a")));
        let id = mesh.spiffe_id_for("svc", "1.2.3.4").unwrap();
        assert_eq!(id, "spiffe://trust.org/ns/team-a/sa/alice");
        assert!(mesh.spiffe_id_for("svc", "9.9.9.9").is_none());
    }
}

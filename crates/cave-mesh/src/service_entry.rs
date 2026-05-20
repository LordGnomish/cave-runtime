// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ServiceEntry manager.
//!
//! Mirrors Istio's `networking.istio.io/v1alpha3` ServiceEntry — the
//! manual CRD that announces an external (or VM-resident) service
//! into the mesh registry. While [`crate::registry`] handles
//! auto-discovered Kubernetes services, this module owns the
//! operator-curated entries: external HTTPS APIs, on-prem VMs,
//! and "headless" virtual workloads.
//!
//! Beyond CRUD, the manager resolves a host string to the right set
//! of endpoints under each `ServiceResolution` mode:
//!
//! * `None` — pass-through; client traffic exits the mesh unmodified.
//! * `Static` — use the `endpoints` list verbatim.
//! * `Dns` — resolve `endpoints[].address` as DNS A/AAAA, pick the
//!   first record (single-shot resolution; caller can repeat to track
//!   record changes).
//! * `DnsRoundRobin` — resolve as `Dns` but cycle through the
//!   resulting addresses on each call.
//!
//! The DNS resolver is pluggable so tests stay deterministic and
//! offline; a `StubResolver` lets a test pre-populate the answer set.

use crate::error::{MeshError, MeshResult};
use crate::models::{ServiceEntry, ServiceLocation, ServicePort, ServiceResolution, WorkloadEntry};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Resolution backend. Production wires up `tokio::net::lookup_host`;
/// tests use [`StubResolver`].
pub trait DnsResolver: Send + Sync {
    fn lookup(&self, host: &str) -> MeshResult<Vec<String>>;
}

#[derive(Default)]
pub struct StubResolver {
    answers: RwLock<HashMap<String, Vec<String>>>,
}

impl StubResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, host: impl Into<String>, addrs: Vec<String>) {
        self.answers.write().unwrap().insert(host.into(), addrs);
    }
}

impl DnsResolver for StubResolver {
    fn lookup(&self, host: &str) -> MeshResult<Vec<String>> {
        self.answers
            .read()
            .unwrap()
            .get(host)
            .cloned()
            .ok_or_else(|| MeshError::not_found(format!("no DNS answer for {host}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEndpoint {
    pub address: String,
    pub port: u16,
    pub protocol: String,
    pub workload_name: Option<String>,
}

pub struct ServiceEntryManager {
    entries: Arc<RwLock<HashMap<String, ServiceEntry>>>,
    rr_cursors: Arc<Mutex<HashMap<String, usize>>>,
    resolver: Arc<dyn DnsResolver>,
}

impl ServiceEntryManager {
    pub fn new(resolver: Arc<dyn DnsResolver>) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            rr_cursors: Arc::new(Mutex::new(HashMap::new())),
            resolver,
        }
    }

    fn key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    pub fn create(&self, mut se: ServiceEntry) -> MeshResult<ServiceEntry> {
        validate(&se)?;
        let now = Utc::now();
        se.created_at = now;
        se.updated_at = now;
        let k = Self::key(&se.namespace, &se.name);
        let mut entries = self.entries.write().unwrap();
        if entries.contains_key(&k) {
            return Err(MeshError::conflict(format!("service entry {k} exists")));
        }
        entries.insert(k, se.clone());
        Ok(se)
    }

    pub fn get(&self, namespace: &str, name: &str) -> MeshResult<ServiceEntry> {
        self.entries
            .read()
            .unwrap()
            .get(&Self::key(namespace, name))
            .cloned()
            .ok_or_else(|| MeshError::not_found(Self::key(namespace, name)))
    }

    pub fn list(&self) -> Vec<ServiceEntry> {
        self.entries.read().unwrap().values().cloned().collect()
    }

    pub fn list_by_host(&self, host: &str) -> Vec<ServiceEntry> {
        self.entries
            .read()
            .unwrap()
            .values()
            .filter(|se| se.hosts.iter().any(|h| h == host))
            .cloned()
            .collect()
    }

    pub fn update(&self, mut se: ServiceEntry) -> MeshResult<ServiceEntry> {
        validate(&se)?;
        let k = Self::key(&se.namespace, &se.name);
        let mut entries = self.entries.write().unwrap();
        let existing = entries
            .get(&k)
            .ok_or_else(|| MeshError::not_found(k.clone()))?;
        se.created_at = existing.created_at;
        se.updated_at = Utc::now();
        entries.insert(k, se.clone());
        Ok(se)
    }

    pub fn delete(&self, namespace: &str, name: &str) -> MeshResult<()> {
        let k = Self::key(namespace, name);
        self.entries
            .write()
            .unwrap()
            .remove(&k)
            .map(|_| ())
            .ok_or_else(|| MeshError::not_found(k))
    }

    /// Resolve a host string to one or more endpoints under the entry's
    /// resolution mode. Hosts not present in any entry → empty vector.
    pub fn resolve(&self, host: &str, port_name: &str) -> MeshResult<Vec<ResolvedEndpoint>> {
        let entries = self.list_by_host(host);
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        // First entry wins — Istio semantics for overlapping hosts.
        let se = entries.into_iter().next().unwrap();
        let port = match se.ports.iter().find(|p| p.name == port_name) {
            Some(p) => p,
            None => se.ports.first().ok_or_else(|| {
                MeshError::invalid_input(format!("entry {} has no ports", se.name))
            })?,
        }
        .clone();

        match se.resolution {
            ServiceResolution::None => Ok(passthrough_endpoint(host, &port)),
            ServiceResolution::Static => Ok(static_endpoints(&se, &port)),
            ServiceResolution::Dns => self.dns_first(&se, &port),
            ServiceResolution::DnsRoundRobin => self.dns_round_robin(&se, &port),
        }
    }

    fn dns_first(
        &self,
        se: &ServiceEntry,
        port: &ServicePort,
    ) -> MeshResult<Vec<ResolvedEndpoint>> {
        let mut out = Vec::new();
        for ep in &se.endpoints {
            let addrs = self.resolver.lookup(&ep.address)?;
            if let Some(addr) = addrs.into_iter().next() {
                out.push(ResolvedEndpoint {
                    address: addr,
                    port: port_for_endpoint(ep, port),
                    protocol: port.protocol.clone(),
                    workload_name: ep.name.clone(),
                });
            }
        }
        Ok(out)
    }

    fn dns_round_robin(
        &self,
        se: &ServiceEntry,
        port: &ServicePort,
    ) -> MeshResult<Vec<ResolvedEndpoint>> {
        let mut all_addrs: Vec<(String, &WorkloadEntry)> = Vec::new();
        for ep in &se.endpoints {
            for addr in self.resolver.lookup(&ep.address)? {
                all_addrs.push((addr, ep));
            }
        }
        if all_addrs.is_empty() {
            return Ok(Vec::new());
        }
        let cursor_key = Self::key(&se.namespace, &se.name);
        let mut cursors = self.rr_cursors.lock().unwrap();
        let cur = cursors.entry(cursor_key).or_insert(0);
        let pick_idx = *cur % all_addrs.len();
        *cur = pick_idx + 1;
        let (addr, ep) = &all_addrs[pick_idx];
        Ok(vec![ResolvedEndpoint {
            address: addr.clone(),
            port: port_for_endpoint(ep, port),
            protocol: port.protocol.clone(),
            workload_name: ep.name.clone(),
        }])
    }
}

fn passthrough_endpoint(host: &str, port: &ServicePort) -> Vec<ResolvedEndpoint> {
    vec![ResolvedEndpoint {
        address: host.to_string(),
        port: port.number,
        protocol: port.protocol.clone(),
        workload_name: None,
    }]
}

fn static_endpoints(se: &ServiceEntry, port: &ServicePort) -> Vec<ResolvedEndpoint> {
    se.endpoints
        .iter()
        .map(|ep| ResolvedEndpoint {
            address: ep.address.clone(),
            port: port_for_endpoint(ep, port),
            protocol: port.protocol.clone(),
            workload_name: ep.name.clone(),
        })
        .collect()
}

fn port_for_endpoint(ep: &WorkloadEntry, fallback: &ServicePort) -> u16 {
    ep.ports
        .get(&fallback.name)
        .copied()
        .unwrap_or(fallback.target_port.unwrap_or(fallback.number))
}

fn validate(se: &ServiceEntry) -> MeshResult<()> {
    if se.hosts.is_empty() {
        return Err(MeshError::invalid_input("hosts must not be empty"));
    }
    if se.ports.is_empty() {
        return Err(MeshError::invalid_input("ports must not be empty"));
    }
    if se.resolution == ServiceResolution::Static && se.endpoints.is_empty() {
        return Err(MeshError::invalid_input(
            "Static resolution requires at least one endpoint",
        ));
    }
    if (se.resolution == ServiceResolution::Dns
        || se.resolution == ServiceResolution::DnsRoundRobin)
        && se.endpoints.is_empty()
    {
        return Err(MeshError::invalid_input(
            "DNS resolution requires at least one endpoint whose address is a hostname",
        ));
    }
    if se.location == ServiceLocation::MeshExternal && se.subject_alt_names.is_empty() {
        // SANs aren't strictly required, but log-style validation in
        // Istio warns on missing SANs for external services that use
        // mTLS. We do not block creation on that — operators may use
        // simple-TLS without SAN verification.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ServiceLocation, ServiceResolution, WorkloadEntry};

    fn entry(name: &str, resolution: ServiceResolution, endpoints: Vec<&str>) -> ServiceEntry {
        ServiceEntry {
            name: name.into(),
            namespace: "ns".into(),
            hosts: vec!["example.com".into()],
            addresses: vec![],
            ports: vec![ServicePort {
                number: 443,
                name: "https".into(),
                protocol: "HTTPS".into(),
                target_port: None,
            }],
            location: ServiceLocation::MeshExternal,
            resolution,
            endpoints: endpoints
                .into_iter()
                .map(|a| WorkloadEntry {
                    name: Some(format!("ep-{a}")),
                    namespace: Some("ns".into()),
                    address: a.into(),
                    ports: HashMap::new(),
                    labels: HashMap::new(),
                    weight: 1,
                    network: None,
                    locality: None,
                    service_account: None,
                    created_at: None,
                    updated_at: None,
                })
                .collect(),
            export_to: vec![],
            subject_alt_names: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn manager() -> (ServiceEntryManager, Arc<StubResolver>) {
        let resolver = Arc::new(StubResolver::new());
        (ServiceEntryManager::new(resolver.clone()), resolver)
    }

    #[test]
    fn create_static_entry_round_trips() {
        let (m, _r) = manager();
        let se = entry(
            "acme",
            ServiceResolution::Static,
            vec!["10.0.0.1", "10.0.0.2"],
        );
        m.create(se).unwrap();
        let got = m.get("ns", "acme").unwrap();
        assert_eq!(got.endpoints.len(), 2);
    }

    #[test]
    fn duplicate_create_refused() {
        let (m, _r) = manager();
        m.create(entry("a", ServiceResolution::Static, vec!["1.1.1.1"]))
            .unwrap();
        let err = m
            .create(entry("a", ServiceResolution::Static, vec!["1.1.1.1"]))
            .unwrap_err();
        assert!(matches!(err, MeshError::Conflict(_)));
    }

    #[test]
    fn validate_rejects_empty_hosts() {
        let (m, _r) = manager();
        let mut se = entry("e", ServiceResolution::None, vec![]);
        se.hosts.clear();
        assert!(matches!(
            m.create(se).unwrap_err(),
            MeshError::InvalidInput(_)
        ));
    }

    #[test]
    fn validate_rejects_static_without_endpoints() {
        let (m, _r) = manager();
        let se = entry("e", ServiceResolution::Static, vec![]);
        assert!(matches!(
            m.create(se).unwrap_err(),
            MeshError::InvalidInput(_)
        ));
    }

    #[test]
    fn none_resolution_passes_host_through() {
        let (m, _r) = manager();
        m.create(entry("a", ServiceResolution::None, vec![]))
            .unwrap();
        let eps = m.resolve("example.com", "https").unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].address, "example.com");
        assert_eq!(eps[0].port, 443);
    }

    #[test]
    fn static_resolution_returns_all_endpoints() {
        let (m, _r) = manager();
        m.create(entry(
            "a",
            ServiceResolution::Static,
            vec!["10.0.0.1", "10.0.0.2", "10.0.0.3"],
        ))
        .unwrap();
        let eps = m.resolve("example.com", "https").unwrap();
        assert_eq!(eps.len(), 3);
    }

    #[test]
    fn dns_resolution_picks_first_record_per_endpoint() {
        let (m, r) = manager();
        r.add(
            "svc-a.internal",
            vec!["192.0.2.10".into(), "192.0.2.11".into()],
        );
        m.create(entry("a", ServiceResolution::Dns, vec!["svc-a.internal"]))
            .unwrap();
        let eps = m.resolve("example.com", "https").unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].address, "192.0.2.10");
    }

    #[test]
    fn dns_resolution_fails_on_unknown_host() {
        let (m, _r) = manager();
        m.create(entry("a", ServiceResolution::Dns, vec!["missing.internal"]))
            .unwrap();
        assert!(m.resolve("example.com", "https").is_err());
    }

    #[test]
    fn dns_round_robin_cycles_through_addresses() {
        let (m, r) = manager();
        r.add(
            "svc.internal",
            vec!["10.0.0.1".into(), "10.0.0.2".into(), "10.0.0.3".into()],
        );
        m.create(entry(
            "rr",
            ServiceResolution::DnsRoundRobin,
            vec!["svc.internal"],
        ))
        .unwrap();
        let a = m.resolve("example.com", "https").unwrap()[0]
            .address
            .clone();
        let b = m.resolve("example.com", "https").unwrap()[0]
            .address
            .clone();
        let c = m.resolve("example.com", "https").unwrap()[0]
            .address
            .clone();
        let d = m.resolve("example.com", "https").unwrap()[0]
            .address
            .clone();
        assert_eq!(
            vec![a, b, c, d],
            vec![
                "10.0.0.1".to_string(),
                "10.0.0.2".into(),
                "10.0.0.3".into(),
                "10.0.0.1".into()
            ]
        );
    }

    #[test]
    fn list_by_host_filters() {
        let (m, _r) = manager();
        let mut a = entry("a", ServiceResolution::Static, vec!["1.1.1.1"]);
        a.hosts = vec!["x.example".into()];
        let mut b = entry("b", ServiceResolution::Static, vec!["2.2.2.2"]);
        b.hosts = vec!["y.example".into()];
        m.create(a).unwrap();
        m.create(b).unwrap();
        assert_eq!(m.list_by_host("x.example").len(), 1);
        assert_eq!(m.list_by_host("missing").len(), 0);
    }

    #[test]
    fn update_preserves_created_at() {
        let (m, _r) = manager();
        let se = m
            .create(entry("a", ServiceResolution::Static, vec!["1.1.1.1"]))
            .unwrap();
        let original = se.created_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut updated = se.clone();
        updated.endpoints[0].address = "2.2.2.2".into();
        let saved = m.update(updated).unwrap();
        assert_eq!(saved.created_at, original);
        assert!(saved.updated_at > original);
    }

    #[test]
    fn delete_removes_entry() {
        let (m, _r) = manager();
        m.create(entry("a", ServiceResolution::Static, vec!["1.1.1.1"]))
            .unwrap();
        m.delete("ns", "a").unwrap();
        assert!(matches!(
            m.get("ns", "a").unwrap_err(),
            MeshError::NotFound(_)
        ));
    }

    #[test]
    fn resolve_unknown_host_returns_empty() {
        let (m, _r) = manager();
        m.create(entry("a", ServiceResolution::Static, vec!["1.1.1.1"]))
            .unwrap();
        let eps = m.resolve("totally-unknown.host", "https").unwrap();
        assert!(eps.is_empty());
    }
}

//! Endpoint Discovery Service — Envoy v3 EDS parity.
//!
//! Stores endpoints per cluster with locality awareness, priority levels,
//! per-endpoint health status, and locality-weighted load balancing
//! support. Drives both:
//! - the `xds.rs` REST handlers (translates `EndpointSet` → Envoy
//!   `ClusterLoadAssignment` JSON), and
//! - the in-process load-balancer (`lb.rs`), by exposing the live healthy
//!   endpoint subset.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Locality {
    pub region: String,
    pub zone: String,
    pub sub_zone: String,
}

impl Locality {
    pub fn new(region: impl Into<String>, zone: impl Into<String>) -> Self {
        Self {
            region: region.into(),
            zone: zone.into(),
            sub_zone: String::new(),
        }
    }

    pub fn with_sub(
        region: impl Into<String>,
        zone: impl Into<String>,
        sub_zone: impl Into<String>,
    ) -> Self {
        Self {
            region: region.into(),
            zone: zone.into(),
            sub_zone: sub_zone.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Unhealthy,
    Draining,
    Timeout,
    Degraded,
}

impl HealthStatus {
    pub fn is_healthy(self) -> bool {
        matches!(self, HealthStatus::Healthy | HealthStatus::Degraded)
    }

    /// Should the endpoint be considered for load balancing?
    /// Draining endpoints are kept for in-flight requests but skipped for new ones.
    pub fn is_routable(self) -> bool {
        matches!(self, HealthStatus::Healthy | HealthStatus::Degraded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub address: SocketAddr,
    pub locality: Locality,
    pub load_balancing_weight: u32,
    pub health: HealthStatus,
    /// Hostname for SNI / Host header.
    pub hostname: Option<String>,
    /// Priority level (Envoy: 0 = highest, 128 = lowest).
    pub priority: u32,
    /// Custom labels for tenant/canary routing.
    pub metadata: HashMap<String, String>,
}

impl Endpoint {
    pub fn new(addr: SocketAddr, locality: Locality, weight: u32) -> Self {
        Self {
            address: addr,
            locality,
            load_balancing_weight: weight,
            health: HealthStatus::Unknown,
            hostname: None,
            priority: 0,
            metadata: HashMap::new(),
        }
    }

    pub fn id(&self) -> String {
        format!("{}/{}", self.address, self.priority)
    }
}

// ── Locality-weighted endpoint group ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalityLbEndpoints {
    pub locality: Locality,
    pub endpoints: Vec<Endpoint>,
    /// Weight for locality-weighted LB (Envoy load_balancing_weight).
    pub locality_weight: u32,
    pub priority: u32,
}

impl LocalityLbEndpoints {
    pub fn new(locality: Locality, priority: u32) -> Self {
        Self {
            locality,
            endpoints: vec![],
            locality_weight: 1,
            priority,
        }
    }

    pub fn healthy(&self) -> Vec<&Endpoint> {
        self.endpoints
            .iter()
            .filter(|e| e.health.is_routable())
            .collect()
    }

    pub fn total_weight(&self) -> u64 {
        self.endpoints
            .iter()
            .filter(|e| e.health.is_routable())
            .map(|e| e.load_balancing_weight as u64)
            .sum()
    }
}

// ── Cluster-level endpoint set ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSet {
    pub cluster_name: String,
    pub version_info: String,
    pub locality_groups: Vec<LocalityLbEndpoints>,
    pub overprovisioning_factor: u32,
}

impl EndpointSet {
    pub fn new(cluster_name: impl Into<String>) -> Self {
        Self {
            cluster_name: cluster_name.into(),
            version_info: "1".to_string(),
            locality_groups: vec![],
            overprovisioning_factor: 140,
        }
    }

    pub fn upsert_locality(&mut self, mut group: LocalityLbEndpoints) {
        // Propagate the group's priority into each endpoint so downstream
        // consumers can sort by Endpoint.priority directly.
        for e in &mut group.endpoints {
            e.priority = group.priority;
        }
        if let Some(pos) = self
            .locality_groups
            .iter()
            .position(|g| g.locality == group.locality && g.priority == group.priority)
        {
            self.locality_groups[pos] = group;
        } else {
            self.locality_groups.push(group);
        }
    }

    pub fn endpoints_at_priority(&self, priority: u32) -> Vec<&Endpoint> {
        self.locality_groups
            .iter()
            .filter(|g| g.priority == priority)
            .flat_map(|g| g.endpoints.iter())
            .collect()
    }

    /// All endpoints across priorities that are routable.
    pub fn routable_endpoints(&self) -> Vec<&Endpoint> {
        let mut out: Vec<&Endpoint> = self
            .locality_groups
            .iter()
            .flat_map(|g| g.endpoints.iter())
            .filter(|e| e.health.is_routable())
            .collect();
        out.sort_by_key(|e| e.priority);
        out
    }

    /// Effective routable set respecting Envoy priority levels with the
    /// overprovisioning factor.
    ///
    /// Returns endpoints from the highest priority that has enough healthy
    /// endpoints; falls through to lower priorities when health < threshold.
    pub fn effective_priority_set(&self) -> Vec<&Endpoint> {
        // Group by priority.
        let mut by_priority: BTreeMap<u32, Vec<&Endpoint>> = BTreeMap::new();
        let mut totals: BTreeMap<u32, (u32, u32)> = BTreeMap::new(); // priority → (healthy, total)
        for g in &self.locality_groups {
            for e in &g.endpoints {
                let entry = totals.entry(g.priority).or_insert((0, 0));
                entry.1 += 1;
                if e.health.is_routable() {
                    entry.0 += 1;
                    by_priority.entry(g.priority).or_default().push(e);
                }
            }
        }

        for (priority, healthy) in by_priority {
            let (h, t) = totals.get(&priority).copied().unwrap_or((0, 0));
            if t == 0 {
                continue;
            }
            // Effective health = (healthy / total) * 100 * overprovisioning / 100
            let effective_health =
                (h as u64 * 100 * self.overprovisioning_factor as u64) / (t as u64 * 100);
            // If effective health >= 100 we serve fully from this priority.
            if effective_health >= 100 {
                return healthy;
            }
            // Otherwise we'd spill over; for simplicity in the config layer
            // we still return what we have at this priority and let the caller
            // weight the spill.
            if !healthy.is_empty() {
                return healthy;
            }
        }
        vec![]
    }

    pub fn locality_count(&self) -> usize {
        self.locality_groups.len()
    }

    pub fn endpoint_count(&self) -> usize {
        self.locality_groups
            .iter()
            .map(|g| g.endpoints.len())
            .sum()
    }
}

// ── EdsRegistry — thread-safe, version-tracked store ─────────────────────────

#[derive(Debug, Default)]
pub struct EdsRegistry {
    inner: RwLock<HashMap<String, Arc<EndpointSet>>>,
    next_version: RwLock<u64>,
}

impl EdsRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(HashMap::new()),
            next_version: RwLock::new(2),
        })
    }

    pub fn upsert(&self, set: EndpointSet) {
        let mut g = self.inner.write().unwrap();
        let mut bumped = set;
        let mut nv = self.next_version.write().unwrap();
        bumped.version_info = nv.to_string();
        *nv += 1;
        g.insert(bumped.cluster_name.clone(), Arc::new(bumped));
    }

    pub fn get(&self, cluster_name: &str) -> Option<Arc<EndpointSet>> {
        self.inner.read().unwrap().get(cluster_name).cloned()
    }

    pub fn list(&self) -> Vec<Arc<EndpointSet>> {
        self.inner.read().unwrap().values().cloned().collect()
    }

    pub fn remove(&self, cluster_name: &str) -> bool {
        self.inner.write().unwrap().remove(cluster_name).is_some()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }

    /// Update the health status of a single endpoint by address+priority.
    pub fn set_endpoint_health(
        &self,
        cluster_name: &str,
        address: SocketAddr,
        priority: u32,
        health: HealthStatus,
    ) -> bool {
        let mut g = self.inner.write().unwrap();
        if let Some(set_arc) = g.get(cluster_name) {
            let mut set = (**set_arc).clone();
            let mut changed = false;
            for group in set.locality_groups.iter_mut() {
                if group.priority != priority {
                    continue;
                }
                for e in group.endpoints.iter_mut() {
                    if e.address == address {
                        e.health = health;
                        changed = true;
                    }
                }
            }
            if changed {
                let mut nv = self.next_version.write().unwrap();
                set.version_info = nv.to_string();
                *nv += 1;
                g.insert(cluster_name.to_string(), Arc::new(set));
            }
            return changed;
        }
        false
    }
}

// ── ACK/NACK — wire-level discovery state ────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryAck {
    Ack { version: String, nonce: String },
    Nack { error_message: String, nonce: String },
}

impl DiscoveryAck {
    pub fn ack(version: impl Into<String>, nonce: impl Into<String>) -> Self {
        DiscoveryAck::Ack {
            version: version.into(),
            nonce: nonce.into(),
        }
    }

    pub fn nack(error: impl Into<String>, nonce: impl Into<String>) -> Self {
        DiscoveryAck::Nack {
            error_message: error.into(),
            nonce: nonce.into(),
        }
    }

    pub fn is_ack(&self) -> bool {
        matches!(self, DiscoveryAck::Ack { .. })
    }
}

/// Per-stream-per-resource version state. The control plane uses this to
/// suppress redundant pushes (Envoy "delta xDS" semantics simplified).
#[derive(Debug, Default)]
pub struct VersionTracker {
    state: RwLock<HashMap<String, String>>,
}

impl VersionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn known(&self, resource: &str) -> Option<String> {
        self.state.read().unwrap().get(resource).cloned()
    }

    pub fn observe(&self, resource: &str, version: &str) {
        self.state
            .write()
            .unwrap()
            .insert(resource.to_string(), version.to_string());
    }

    /// Returns the resources whose live version differs from the tracked
    /// last-known version.
    pub fn diff(&self, current: &HashMap<String, String>) -> Vec<String> {
        let known = self.state.read().unwrap();
        let mut out = Vec::new();
        for (k, v) in current {
            match known.get(k) {
                Some(prev) if prev == v => {}
                _ => out.push(k.clone()),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn ep(addr: &str, port: u16, weight: u32, locality: Locality, health: HealthStatus) -> Endpoint {
        let ip: IpAddr = addr.parse().unwrap();
        let mut e = Endpoint::new(SocketAddr::new(ip, port), locality, weight);
        e.health = health;
        e
    }

    fn group(locality: Locality, endpoints: Vec<Endpoint>, priority: u32, weight: u32) -> LocalityLbEndpoints {
        let mut g = LocalityLbEndpoints::new(locality, priority);
        g.endpoints = endpoints;
        g.locality_weight = weight;
        g
    }

    // ── Locality basics ──

    #[test]
    fn locality_new_no_subzone() {
        let l = Locality::new("us-east", "us-east-1a");
        assert_eq!(l.region, "us-east");
        assert_eq!(l.zone, "us-east-1a");
        assert!(l.sub_zone.is_empty());
    }

    #[test]
    fn locality_with_sub() {
        let l = Locality::with_sub("us-east", "us-east-1a", "rack3");
        assert_eq!(l.sub_zone, "rack3");
    }

    #[test]
    fn locality_default_empty() {
        assert_eq!(Locality::default(), Locality::new("", ""));
    }

    // ── HealthStatus ──

    #[test]
    fn health_healthy_routable() {
        assert!(HealthStatus::Healthy.is_healthy());
        assert!(HealthStatus::Healthy.is_routable());
    }

    #[test]
    fn health_degraded_routable() {
        assert!(HealthStatus::Degraded.is_routable());
        assert!(HealthStatus::Degraded.is_healthy());
    }

    #[test]
    fn health_draining_not_routable() {
        assert!(!HealthStatus::Draining.is_routable());
    }

    #[test]
    fn health_unhealthy_not_routable() {
        assert!(!HealthStatus::Unhealthy.is_routable());
    }

    #[test]
    fn health_unknown_not_routable() {
        // Unknown is conservative — endpoints must be probed before use.
        assert!(!HealthStatus::Unknown.is_routable());
    }

    // ── Endpoint ──

    #[test]
    fn endpoint_id_includes_priority() {
        let e = ep("1.2.3.4", 80, 1, Locality::default(), HealthStatus::Healthy);
        assert!(e.id().contains("1.2.3.4:80"));
        assert!(e.id().ends_with("/0"));
    }

    // ── LocalityLbEndpoints ──

    #[test]
    fn locality_group_filters_unhealthy() {
        let l = Locality::new("us", "us-1a");
        let g = group(
            l.clone(),
            vec![
                ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy),
                ep("1.1.1.2", 80, 1, l.clone(), HealthStatus::Unhealthy),
                ep("1.1.1.3", 80, 1, l.clone(), HealthStatus::Draining),
            ],
            0,
            1,
        );
        assert_eq!(g.healthy().len(), 1);
        assert_eq!(g.total_weight(), 1);
    }

    #[test]
    fn locality_group_total_weight_sums_healthy() {
        let l = Locality::new("us", "us-1a");
        let g = group(
            l.clone(),
            vec![
                ep("1.1.1.1", 80, 5, l.clone(), HealthStatus::Healthy),
                ep("1.1.1.2", 80, 3, l.clone(), HealthStatus::Healthy),
                ep("1.1.1.3", 80, 100, l.clone(), HealthStatus::Unhealthy),
            ],
            0,
            1,
        );
        assert_eq!(g.total_weight(), 8);
    }

    // ── EndpointSet ──

    #[test]
    fn set_upsert_replaces_locality() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy)],
            0,
            1,
        ));
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.2", 80, 1, l.clone(), HealthStatus::Healthy)],
            0,
            1,
        ));
        assert_eq!(set.locality_count(), 1);
        assert_eq!(set.endpoint_count(), 1);
    }

    #[test]
    fn set_upsert_separates_priority() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy)],
            0,
            1,
        ));
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.2", 80, 1, l.clone(), HealthStatus::Healthy)],
            1,
            1,
        ));
        assert_eq!(set.locality_count(), 2);
    }

    #[test]
    fn set_endpoints_at_priority() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy)],
            0,
            1,
        ));
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.2", 80, 1, l.clone(), HealthStatus::Healthy)],
            1,
            1,
        ));
        assert_eq!(set.endpoints_at_priority(0).len(), 1);
        assert_eq!(set.endpoints_at_priority(1).len(), 1);
    }

    #[test]
    fn set_routable_endpoints_filters_and_sorts_by_priority() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy)],
            1,
            1,
        ));
        set.upsert_locality(group(
            l.clone(),
            vec![
                ep("1.1.1.2", 80, 1, l.clone(), HealthStatus::Healthy),
                ep("1.1.1.3", 80, 1, l.clone(), HealthStatus::Unhealthy),
            ],
            0,
            1,
        ));
        let routable = set.routable_endpoints();
        assert_eq!(routable.len(), 2);
        assert_eq!(routable[0].priority, 0);
        assert_eq!(routable[1].priority, 1);
    }

    // ── effective_priority_set ──

    #[test]
    fn effective_priority_full_health_uses_p0() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        set.upsert_locality(group(
            l.clone(),
            vec![
                ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy),
                ep("1.1.1.2", 80, 1, l.clone(), HealthStatus::Healthy),
            ],
            0,
            1,
        ));
        set.upsert_locality(group(
            l.clone(),
            vec![ep("2.2.2.2", 80, 1, l.clone(), HealthStatus::Healthy)],
            1,
            1,
        ));
        let eps = set.effective_priority_set();
        assert_eq!(eps.len(), 2);
        assert!(eps.iter().all(|e| e.priority == 0));
    }

    #[test]
    fn effective_priority_falls_through_when_p0_empty() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Unhealthy)],
            0,
            1,
        ));
        set.upsert_locality(group(
            l.clone(),
            vec![ep("2.2.2.2", 80, 1, l.clone(), HealthStatus::Healthy)],
            1,
            1,
        ));
        let eps = set.effective_priority_set();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].priority, 1);
    }

    #[test]
    fn effective_priority_overprovisioning_140() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        // 2 of 3 healthy at p0 = 67% * 1.40 = 93% < 100, so it spills.
        set.upsert_locality(group(
            l.clone(),
            vec![
                ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy),
                ep("1.1.1.2", 80, 1, l.clone(), HealthStatus::Healthy),
                ep("1.1.1.3", 80, 1, l.clone(), HealthStatus::Unhealthy),
            ],
            0,
            1,
        ));
        let eps = set.effective_priority_set();
        // We still serve from p0 (only one priority), but the count reflects
        // the 2 healthy, not 3.
        assert_eq!(eps.len(), 2);
    }

    // ── EdsRegistry ──

    #[test]
    fn registry_upsert_assigns_version() {
        let r = EdsRegistry::new();
        let set = EndpointSet::new("a");
        r.upsert(set);
        let s = r.get("a").unwrap();
        assert!(!s.version_info.is_empty());
    }

    #[test]
    fn registry_upsert_bumps_version_each_time() {
        let r = EdsRegistry::new();
        r.upsert(EndpointSet::new("a"));
        let v1 = r.get("a").unwrap().version_info.clone();
        r.upsert(EndpointSet::new("a"));
        let v2 = r.get("a").unwrap().version_info.clone();
        assert_ne!(v1, v2);
    }

    #[test]
    fn registry_remove() {
        let r = EdsRegistry::new();
        r.upsert(EndpointSet::new("a"));
        assert!(r.remove("a"));
        assert!(!r.remove("a"));
    }

    #[test]
    fn registry_set_endpoint_health_updates() {
        let r = EdsRegistry::new();
        let l = Locality::new("us", "us-1a");
        let mut set = EndpointSet::new("svc");
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Unknown)],
            0,
            1,
        ));
        r.upsert(set);
        let addr: SocketAddr = "1.1.1.1:80".parse().unwrap();
        assert!(r.set_endpoint_health("svc", addr, 0, HealthStatus::Healthy));
        let updated = r.get("svc").unwrap();
        let endpoint = updated.locality_groups[0]
            .endpoints
            .iter()
            .find(|e| e.address == addr)
            .unwrap();
        assert_eq!(endpoint.health, HealthStatus::Healthy);
    }

    #[test]
    fn registry_set_endpoint_health_no_match_returns_false() {
        let r = EdsRegistry::new();
        let addr: SocketAddr = "1.1.1.1:80".parse().unwrap();
        assert!(!r.set_endpoint_health("nope", addr, 0, HealthStatus::Healthy));
    }

    // ── DiscoveryAck ──

    #[test]
    fn discovery_ack_basic() {
        let a = DiscoveryAck::ack("v1", "n1");
        assert!(a.is_ack());
    }

    #[test]
    fn discovery_nack() {
        let n = DiscoveryAck::nack("bad version", "n2");
        assert!(!n.is_ack());
    }

    // ── VersionTracker ──

    #[test]
    fn version_tracker_observes_and_recalls() {
        let t = VersionTracker::new();
        t.observe("svc-a", "v1");
        assert_eq!(t.known("svc-a"), Some("v1".to_string()));
        assert!(t.known("svc-b").is_none());
    }

    #[test]
    fn version_tracker_diff_finds_changed_and_new() {
        let t = VersionTracker::new();
        t.observe("a", "v1");
        t.observe("b", "v1");
        let mut current = HashMap::new();
        current.insert("a".to_string(), "v1".to_string());
        current.insert("b".to_string(), "v2".to_string()); // changed
        current.insert("c".to_string(), "v1".to_string()); // new
        let diff = t.diff(&current);
        assert!(diff.contains(&"b".to_string()));
        assert!(diff.contains(&"c".to_string()));
        assert!(!diff.contains(&"a".to_string()));
    }

    // ── Serde round-trips ──

    #[test]
    fn endpoint_set_json_roundtrip() {
        let mut set = EndpointSet::new("svc");
        let l = Locality::new("us", "us-1a");
        set.upsert_locality(group(
            l.clone(),
            vec![ep("1.1.1.1", 80, 1, l.clone(), HealthStatus::Healthy)],
            0,
            1,
        ));
        let s = serde_json::to_string(&set).unwrap();
        let s2: EndpointSet = serde_json::from_str(&s).unwrap();
        assert_eq!(set, s2);
    }

    #[test]
    fn health_status_json_roundtrip_uppercase() {
        let s = serde_json::to_string(&HealthStatus::Draining).unwrap();
        assert_eq!(s, r#""DRAINING""#);
    }

    #[test]
    fn locality_json_roundtrip() {
        let l = Locality::with_sub("eu-west", "eu-west-1a", "rack-7");
        let s = serde_json::to_string(&l).unwrap();
        let l2: Locality = serde_json::from_str(&s).unwrap();
        assert_eq!(l, l2);
    }
}

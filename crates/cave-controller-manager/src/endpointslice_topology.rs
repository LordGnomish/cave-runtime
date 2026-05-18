// SPDX-License-Identifier: AGPL-3.0-or-later
//! EndpointSlice topology-aware hints — `staging/src/k8s.io/endpointslice/topologycache/topologycache.go`.
//!
//! When `Service.spec.trafficDistribution = "PreferClose"` (or the legacy
//! `service.kubernetes.io/topology-mode: Auto` annotation), the controller
//! injects `endpoints[].hints.forZones[]` so kube-proxy / kpng prefers the
//! local zone. Algorithm:
//!
//! 1. Skip if total ready endpoints < `min_endpoints_per_zone * zone_count`
//!    (default `MinEndpointsPerZone = 7`).
//! 2. Skip if endpoints aren't present in at least 2 zones.
//! 3. Compute per-zone capacity proportional to zone CPU share.
//! 4. Assign hints zone-by-zone, capping each zone at its capacity.
//!
//! Mirrors `RemoveHintsFromSlices` (when disabled) and
//! `redistributeHintsByZone` (when enabled).

use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Minimum ready endpoints per zone for the algorithm to engage.
/// Mirrors `topologycache.MinEndpointsPerZone`.
pub const MIN_ENDPOINTS_PER_ZONE: u32 = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    pub name: String,
    /// Sum of allocatable CPU across nodes in this zone, milli-cores.
    pub cpu_milli: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyEndpoint {
    pub address: String,
    pub zone: String,
    pub ready: bool,
}

/// Hint emitted onto an endpoint: which zones may consume it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyHint {
    pub address: String,
    pub for_zones: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopologyDecision {
    /// Algorithm engaged — hints[] holds per-endpoint assignments.
    Engaged(Vec<TopologyHint>),
    /// Skipped because cluster lacks zone diversity or endpoint count.
    Disabled(String),
}

/// Return zone → ready-endpoint count map.
fn zone_endpoint_counts(endpoints: &[ReadyEndpoint]) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    for e in endpoints {
        if !e.ready {
            continue;
        }
        *m.entry(e.zone.clone()).or_insert(0) += 1;
    }
    m
}

/// Run the topology-aware hint computation.
pub fn compute_hints(
    endpoints: &[ReadyEndpoint],
    zones: &[ZoneInfo],
    min_endpoints_per_zone: u32,
) -> TopologyDecision {
    let counts = zone_endpoint_counts(endpoints);
    if counts.len() < 2 {
        return TopologyDecision::Disabled("fewer than 2 zones with ready endpoints".into());
    }
    let total_ready: u32 = counts.values().copied().sum();
    let required = min_endpoints_per_zone * counts.len() as u32;
    if total_ready < required {
        return TopologyDecision::Disabled("insufficient ready endpoints across zones".into());
    }
    // Per-zone capacity proportional to zone CPU share — but only counting
    // zones that actually have ready endpoints (matches upstream
    // `getCPUByZone`).
    let total_cpu: u64 = zones
        .iter()
        .filter(|z| counts.contains_key(&z.name))
        .map(|z| z.cpu_milli)
        .sum();
    if total_cpu == 0 {
        return TopologyDecision::Disabled("no CPU info for any zone".into());
    }

    // Each zone's expected share rounded.
    let mut per_zone_target: BTreeMap<String, u32> = BTreeMap::new();
    let mut allocated: u32 = 0;
    for z in zones {
        if !counts.contains_key(&z.name) {
            continue;
        }
        // proportional rounding
        let share =
            ((z.cpu_milli as u128) * (total_ready as u128) / total_cpu as u128) as u32;
        per_zone_target.insert(z.name.clone(), share);
        allocated += share;
    }
    // Distribute the rounding remainder to zones with the highest CPU.
    let mut leftover = total_ready.saturating_sub(allocated);
    if leftover > 0 {
        let mut by_cpu: Vec<&ZoneInfo> = zones
            .iter()
            .filter(|z| counts.contains_key(&z.name))
            .collect();
        by_cpu.sort_by_key(|z| std::cmp::Reverse(z.cpu_milli));
        for z in by_cpu {
            if leftover == 0 {
                break;
            }
            *per_zone_target.entry(z.name.clone()).or_insert(0) += 1;
            leftover -= 1;
        }
    }

    // Now assign each ready endpoint a forZones list. The simple model:
    // forZones = [endpoint.zone] for the first `target` endpoints in that
    // zone; surplus endpoints get a fallback assignment to the
    // most-capacity-deficient zone (overflow).
    let mut hints = Vec::new();
    let mut assigned_per_zone: BTreeMap<String, u32> = BTreeMap::new();
    // Iterate in stable order for determinism.
    let mut sorted: Vec<&ReadyEndpoint> = endpoints.iter().filter(|e| e.ready).collect();
    sorted.sort_by(|a, b| a.address.cmp(&b.address));
    for ep in sorted {
        let used = *assigned_per_zone.get(&ep.zone).unwrap_or(&0);
        let target = *per_zone_target.get(&ep.zone).unwrap_or(&0);
        if used < target {
            hints.push(TopologyHint {
                address: ep.address.clone(),
                for_zones: vec![ep.zone.clone()],
            });
            *assigned_per_zone.entry(ep.zone.clone()).or_insert(0) += 1;
        } else {
            // Surplus: pick a deficit zone.
            let deficit_zone = per_zone_target
                .iter()
                .filter_map(|(z, target)| {
                    let used = *assigned_per_zone.get(z).unwrap_or(&0);
                    if used < *target {
                        Some((z.clone(), *target - used))
                    } else {
                        None
                    }
                })
                .max_by_key(|(_, deficit)| *deficit)
                .map(|(z, _)| z);
            match deficit_zone {
                Some(target_zone) => {
                    hints.push(TopologyHint {
                        address: ep.address.clone(),
                        for_zones: vec![target_zone.clone()],
                    });
                    *assigned_per_zone.entry(target_zone).or_insert(0) += 1;
                }
                None => {
                    // No deficit zone — emit hint for endpoint's own zone.
                    hints.push(TopologyHint {
                        address: ep.address.clone(),
                        for_zones: vec![ep.zone.clone()],
                    });
                }
            }
        }
    }
    TopologyDecision::Engaged(hints)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
    "TopologyCache",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ep(addr: &str, zone: &str) -> ReadyEndpoint {
        ReadyEndpoint { address: addr.into(), zone: zone.into(), ready: true }
    }
    fn zinfo(name: &str, cpu: u64) -> ZoneInfo {
        ZoneInfo { name: name.into(), cpu_milli: cpu }
    }

    #[test]
    fn single_zone_disables_topology() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "AddHints",
            "tenant-est-single-zone"
        );
        let eps: Vec<_> = (0..15).map(|i| ep(&format!("p{i}"), "us-east-1a")).collect();
        let zones = vec![zinfo("us-east-1a", 10_000)];
        match compute_hints(&eps, &zones, MIN_ENDPOINTS_PER_ZONE) {
            TopologyDecision::Disabled(_) => {}
            other => panic!("expected Disabled, got {:?}", other),
        }
    }

    #[test]
    fn insufficient_endpoints_across_zones_disables() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "AddHints",
            "tenant-est-too-few"
        );
        let eps = vec![ep("a", "z1"), ep("b", "z2")];
        let zones = vec![zinfo("z1", 1000), zinfo("z2", 1000)];
        // 2 endpoints, 2 zones → required = 2*7 = 14 → disabled.
        match compute_hints(&eps, &zones, MIN_ENDPOINTS_PER_ZONE) {
            TopologyDecision::Disabled(_) => {}
            other => panic!("expected Disabled, got {:?}", other),
        }
    }

    #[test]
    fn equal_zones_distribute_evenly() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "redistributeHintsByZone",
            "tenant-est-equal"
        );
        let mut eps = Vec::new();
        for i in 0..10 {
            eps.push(ep(&format!("a{i}"), "z1"));
            eps.push(ep(&format!("b{i}"), "z2"));
        }
        let zones = vec![zinfo("z1", 1000), zinfo("z2", 1000)];
        let dec = compute_hints(&eps, &zones, 5);
        match dec {
            TopologyDecision::Engaged(hints) => {
                assert_eq!(hints.len(), 20);
            }
            other => panic!("expected Engaged, got {:?}", other),
        }
    }

    #[test]
    fn skewed_capacity_shifts_hints_to_larger_zone() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "redistributeHintsByZone",
            "tenant-est-skew"
        );
        // z1 has 75% of CPU but only 50% of endpoints — surplus endpoints
        // from z2 should be hinted toward z1 to absorb the load.
        let mut eps = Vec::new();
        for i in 0..10 {
            eps.push(ep(&format!("a{i}"), "z1"));
            eps.push(ep(&format!("b{i}"), "z2"));
        }
        let zones = vec![zinfo("z1", 3000), zinfo("z2", 1000)];
        let dec = compute_hints(&eps, &zones, 5);
        match dec {
            TopologyDecision::Engaged(hints) => {
                let z1_hinted =
                    hints.iter().filter(|h| h.for_zones == vec!["z1".to_string()]).count();
                let z2_hinted =
                    hints.iter().filter(|h| h.for_zones == vec!["z2".to_string()]).count();
                // z1 should be hinted by ~75% of endpoints.
                assert!(z1_hinted >= 14);
                assert!(z2_hinted <= 6);
            }
            other => panic!("expected Engaged, got {:?}", other),
        }
    }

    #[test]
    fn unready_endpoints_excluded_from_count() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "AddHints",
            "tenant-est-unready"
        );
        let mut eps = Vec::new();
        for i in 0..3 {
            eps.push(ep(&format!("a{i}"), "z1"));
            eps.push(ep(&format!("b{i}"), "z2"));
        }
        // Unready endpoints don't count toward the threshold.
        for i in 0..20 {
            eps.push(ReadyEndpoint {
                address: format!("u{i}"),
                zone: "z1".into(),
                ready: false,
            });
        }
        let zones = vec![zinfo("z1", 1000), zinfo("z2", 1000)];
        // Threshold per-zone 5 → required 10; ready=6 → disabled.
        match compute_hints(&eps, &zones, 5) {
            TopologyDecision::Disabled(_) => {}
            other => panic!("expected Disabled, got {:?}", other),
        }
    }

    #[test]
    fn missing_zone_cpu_disables() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "redistributeHintsByZone",
            "tenant-est-no-cpu"
        );
        let mut eps = Vec::new();
        for i in 0..15 {
            eps.push(ep(&format!("a{i}"), "z1"));
            eps.push(ep(&format!("b{i}"), "z2"));
        }
        // Both zones have 0 CPU → algorithm cannot proportion.
        let zones = vec![zinfo("z1", 0), zinfo("z2", 0)];
        match compute_hints(&eps, &zones, 5) {
            TopologyDecision::Disabled(_) => {}
            other => panic!("expected Disabled, got {:?}", other),
        }
    }

    #[test]
    fn min_endpoints_per_zone_constant() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "MinEndpointsPerZone",
            "tenant-est-const"
        );
        assert_eq!(MIN_ENDPOINTS_PER_ZONE, 7);
    }

    #[test]
    fn hints_emitted_for_every_ready_endpoint() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "AddHints",
            "tenant-est-coverage"
        );
        let mut eps = Vec::new();
        for i in 0..7 {
            eps.push(ep(&format!("a{i}"), "z1"));
            eps.push(ep(&format!("b{i}"), "z2"));
        }
        let zones = vec![zinfo("z1", 1000), zinfo("z2", 1000)];
        let dec = compute_hints(&eps, &zones, 5);
        match dec {
            TopologyDecision::Engaged(hints) => assert_eq!(hints.len(), 14),
            _ => panic!("expected Engaged"),
        }
    }

    #[test]
    fn topology_decision_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "TopologyDecision",
            "tenant-est-serde"
        );
        let d = TopologyDecision::Engaged(vec![TopologyHint {
            address: "a".into(),
            for_zones: vec!["z1".into()],
        }]);
        let s = serde_json::to_string(&d).unwrap();
        let back: TopologyDecision = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn endpoint_address_is_preserved_in_hint() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/endpointslice/topologycache/topologycache.go",
            "AddHints",
            "tenant-est-addr-preserved"
        );
        let mut eps = Vec::new();
        for i in 0..6 {
            eps.push(ep(&format!("ep-{i}"), "z1"));
            eps.push(ep(&format!("ep-{i}b"), "z2"));
        }
        let zones = vec![zinfo("z1", 500), zinfo("z2", 500)];
        match compute_hints(&eps, &zones, 5) {
            TopologyDecision::Engaged(hints) => {
                for h in hints {
                    assert!(h.address.starts_with("ep-"));
                    assert!(!h.for_zones.is_empty());
                }
            }
            _ => panic!("expected Engaged"),
        }
    }
}

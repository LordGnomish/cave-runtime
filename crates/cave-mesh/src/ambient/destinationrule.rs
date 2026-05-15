// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DestinationRule — cluster definition + load-balancing policy.
//!
//! Mirrors `pilot/pkg/networking/core/v1alpha3/cluster.go::buildDefaultCluster`
//! plus the LB policy switch from `loadbalancer.go`.
//!
//! A DestinationRule has zero-or-more `Subset`s (tag-based slices of the
//! parent host) and one top-level `LoadBalancer`. Compilation produces one
//! `Cluster` per (host, subset) pair, plus the chosen LB policy.

use crate::ambient::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbPolicy {
    RoundRobin,
    LeastRequest,
    Random,
    /// Consistent hash on a header value.
    RingHash { header: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subset {
    pub name: String,
    /// Pod label selector — `(k, v)` pairs that all must match.
    pub labels: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationRule {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub host: String,
    pub lb: LbPolicy,
    pub subsets: Vec<Subset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cluster {
    /// Istio cluster name: `<host>` for the default subset, `<host>|<subset>`
    /// otherwise. Mirrors `model.BuildSubsetKey`.
    pub name: String,
    pub host: String,
    pub subset: String,
    pub lb: LbPolicy,
    pub label_selector: Vec<(String, String)>,
}

/// Backend endpoint with arbitrary labels (so `RingHash` and the subset
/// label-selector tests can run without setting up a real EDS pipeline).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub address: String,
    /// Number of in-flight requests (used by LeastRequest).
    pub active_requests: u32,
    pub labels: Vec<(String, String)>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DrError {
    #[error("DestinationRule {0} has empty host")]
    NoHost(String),
    #[error("subset {0} has empty label selector")]
    EmptySubset(String),
    #[error("no endpoints available for cluster {0}")]
    NoEndpoints(String),
}

/// Compile a DestinationRule to one or more `Cluster`s.
pub fn compile(dr: &DestinationRule) -> Result<Vec<Cluster>, DrError> {
    if dr.host.trim().is_empty() {
        return Err(DrError::NoHost(dr.name.clone()));
    }
    let mut out = vec![Cluster {
        name: dr.host.clone(),
        host: dr.host.clone(),
        subset: String::new(),
        lb: dr.lb.clone(),
        label_selector: vec![],
    }];
    for s in &dr.subsets {
        if s.labels.is_empty() {
            return Err(DrError::EmptySubset(s.name.clone()));
        }
        out.push(Cluster {
            name: format!("{}|{}", dr.host, s.name),
            host: dr.host.clone(),
            subset: s.name.clone(),
            lb: dr.lb.clone(),
            label_selector: s.labels.clone(),
        });
    }
    Ok(out)
}

/// Pick one endpoint per the cluster's LB policy. Mirrors the policy switch
/// in `pilot/pkg/networking/util/loadbalancer.go::ApplyLocalityLBSetting`.
///
/// `req_seed` is the round-robin counter or the consistent-hash key seed,
/// depending on policy. For `RingHash`, `headers` provides the hashed value.
pub fn pick<'a>(
    cluster: &Cluster,
    endpoints: &'a [Endpoint],
    req_seed: u64,
    headers: &[(&str, &str)],
) -> Result<&'a Endpoint, DrError> {
    if endpoints.is_empty() {
        return Err(DrError::NoEndpoints(cluster.name.clone()));
    }
    // Apply subset label selector first.
    let pool: Vec<&Endpoint> = endpoints
        .iter()
        .filter(|e| {
            cluster
                .label_selector
                .iter()
                .all(|(k, v)| e.labels.iter().any(|(ek, ev)| ek == k && ev == v))
        })
        .collect();
    if pool.is_empty() {
        return Err(DrError::NoEndpoints(cluster.name.clone()));
    }

    let chosen = match &cluster.lb {
        LbPolicy::RoundRobin => pool[(req_seed as usize) % pool.len()],
        LbPolicy::LeastRequest => *pool
            .iter()
            .min_by_key(|e| e.active_requests)
            .expect("pool non-empty"),
        LbPolicy::Random => pool[(req_seed as usize) % pool.len()],
        LbPolicy::RingHash { header } => {
            let key = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(header))
                .map(|(_, v)| *v)
                .unwrap_or("");
            // Deterministic, dependency-free hash (FNV-1a 64) keyed on the header.
            let mut h: u64 = 0xcbf29ce484222325;
            for b in key.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            pool[(h as usize) % pool.len()]
        }
    };
    Ok(chosen)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::istio(
    "pilot/pkg/networking/core/v1alpha3/cluster.go",
    "buildDefaultCluster",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn dr(lb: LbPolicy, subsets: Vec<Subset>) -> DestinationRule {
        DestinationRule {
            name: "web-dr".into(),
            namespace: "acme".into(),
            tenant: TenantId::new("acme").expect("test fixture"),
            host: "web.acme.svc.cluster.local".into(),
            lb,
            subsets,
        }
    }

    fn ep(addr: &str, active: u32, labels: &[(&str, &str)]) -> Endpoint {
        Endpoint {
            address: addr.into(),
            active_requests: active,
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn compile_emits_default_plus_subset_clusters() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/cluster.go",
            "buildSubsetCluster",
            "tenant-dr-compile"
        );
        let d = dr(
            LbPolicy::RoundRobin,
            vec![Subset {
                name: "v1".into(),
                labels: vec![("version".into(), "v1".into())],
            }],
        );
        let cs = compile(&d).unwrap();
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].subset, "");
        assert_eq!(cs[0].name, "web.acme.svc.cluster.local");
        assert_eq!(cs[1].subset, "v1");
        assert_eq!(cs[1].name, "web.acme.svc.cluster.local|v1");
    }

    #[test]
    fn empty_subset_selector_is_rejected() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/cluster.go",
            "validateSubset",
            "tenant-dr-bad-subset"
        );
        let d = dr(
            LbPolicy::RoundRobin,
            vec![Subset { name: "v1".into(), labels: vec![] }],
        );
        assert!(matches!(compile(&d), Err(DrError::EmptySubset(_))));
    }

    #[test]
    fn round_robin_picks_by_seed_modulo_pool_size() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/util/loadbalancer.go",
            "ApplyLocalityLBSetting",
            "tenant-dr-rr"
        );
        let cs = compile(&dr(LbPolicy::RoundRobin, vec![])).unwrap();
        let eps = vec![ep("a", 0, &[]), ep("b", 0, &[]), ep("c", 0, &[])];
        assert_eq!(pick(&cs[0], &eps, 0, &[]).unwrap().address, "a");
        assert_eq!(pick(&cs[0], &eps, 1, &[]).unwrap().address, "b");
        assert_eq!(pick(&cs[0], &eps, 4, &[]).unwrap().address, "b");
    }

    #[test]
    fn least_request_picks_the_endpoint_with_fewest_in_flight() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/util/loadbalancer.go",
            "leastRequest",
            "tenant-dr-lr"
        );
        let cs = compile(&dr(LbPolicy::LeastRequest, vec![])).unwrap();
        let eps = vec![ep("a", 5, &[]), ep("b", 1, &[]), ep("c", 3, &[])];
        assert_eq!(pick(&cs[0], &eps, 0, &[]).unwrap().address, "b");
    }

    #[test]
    fn ring_hash_routes_same_key_to_same_endpoint() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/util/loadbalancer.go",
            "applyConsistentHashLB",
            "tenant-dr-ring"
        );
        let cs = compile(&dr(LbPolicy::RingHash { header: "x-user".into() }, vec![])).unwrap();
        let eps = vec![ep("a", 0, &[]), ep("b", 0, &[]), ep("c", 0, &[])];
        let one = pick(&cs[0], &eps, 0, &[("x-user", "alice")]).unwrap().address.clone();
        let two = pick(&cs[0], &eps, 0, &[("x-user", "alice")]).unwrap().address.clone();
        assert_eq!(one, two);
    }

    #[test]
    fn subset_selector_filters_endpoint_pool() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/cluster.go",
            "applySubsetSelector",
            "tenant-dr-subset"
        );
        let cs = compile(&dr(
            LbPolicy::RoundRobin,
            vec![Subset { name: "v1".into(), labels: vec![("version".into(), "v1".into())] }],
        ))
        .unwrap();
        let eps = vec![
            ep("v1-a", 0, &[("version", "v1")]),
            ep("v2-a", 0, &[("version", "v2")]),
            ep("v1-b", 0, &[("version", "v1")]),
        ];
        // Subset cluster (cs[1]) must restrict to v1-* endpoints only.
        let chosen = pick(&cs[1], &eps, 0, &[]).unwrap();
        assert!(chosen.address.starts_with("v1-"));
    }

    #[test]
    fn empty_endpoint_pool_returns_no_endpoints_error() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/util/loadbalancer.go",
            "ApplyLocalityLBSetting",
            "tenant-dr-empty"
        );
        let cs = compile(&dr(LbPolicy::RoundRobin, vec![])).unwrap();
        assert!(matches!(pick(&cs[0], &[], 0, &[]), Err(DrError::NoEndpoints(_))));
    }
}

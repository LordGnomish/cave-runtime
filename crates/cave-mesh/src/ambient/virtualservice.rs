//! VirtualService → ordered route table compiler.
//!
//! Mirrors `pilot/pkg/networking/core/v1alpha3/route/route.go::translateRoutes`.
//! A VirtualService spec is compiled to the ordered, flattened route table
//! the waypoint runtime evaluates; each output route carries a weight pool
//! that adds to 100 (or returns an error otherwise).

use crate::ambient::types::{Cite, TenantId};
use crate::ambient::waypoint::{Backend, Protocol, Route, RouteMatch};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationRef {
    pub host: String,
    /// Subset name (typically a `version` label). Empty = no subset.
    pub subset: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightedDestination {
    pub destination: DestinationRef,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRoute {
    pub name: String,
    pub r#match: RouteMatch,
    pub route: Vec<WeightedDestination>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualService {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub hosts: Vec<String>,
    pub http: Vec<HttpRoute>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CompileError {
    #[error("VirtualService {0} has empty hosts list")]
    NoHosts(String),
    #[error("HttpRoute {route} weights must sum to 100, got {actual}")]
    BadWeightSum { route: String, actual: u32 },
    #[error("HttpRoute {route} has empty destination list")]
    EmptyDestinations { route: String },
}

/// Compile a VirtualService into the flat `Route` shape that
/// `WaypointConfig` consumes. Each weighted destination becomes its own
/// `Route` row, with the weight preserved on `Backend.weight`. The cluster
/// name follows Istio's naming convention: `<host>|<subset>` (subset omitted
/// when empty).
pub fn compile(vs: &VirtualService, default_protocol: Protocol) -> Result<Vec<Route>, CompileError> {
    if vs.hosts.is_empty() {
        return Err(CompileError::NoHosts(vs.name.clone()));
    }
    let mut out = Vec::new();
    for h in &vs.http {
        if h.route.is_empty() {
            return Err(CompileError::EmptyDestinations { route: h.name.clone() });
        }
        let sum: u32 = h.route.iter().map(|w| w.weight).sum();
        if sum != 100 {
            return Err(CompileError::BadWeightSum { route: h.name.clone(), actual: sum });
        }
        for (idx, wd) in h.route.iter().enumerate() {
            let cluster = if wd.destination.subset.is_empty() {
                wd.destination.host.clone()
            } else {
                format!("{}|{}", wd.destination.host, wd.destination.subset)
            };
            out.push(Route {
                name: format!("{}#{}", h.name, idx),
                r#match: h.r#match.clone(),
                backend: Backend { cluster, weight: wd.weight },
                protocol: default_protocol,
            });
        }
    }
    Ok(out)
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::istio("pilot/pkg/networking/core/v1alpha3/route/route.go", "translateRoutes");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn vs(http: Vec<HttpRoute>) -> VirtualService {
        VirtualService {
            name: "web-vs".into(),
            namespace: "acme".into(),
            tenant: TenantId::new("acme").expect("test fixture"),
            hosts: vec!["web.acme.svc.cluster.local".into()],
            http,
        }
    }

    fn dest(host: &str, subset: &str) -> DestinationRef {
        DestinationRef { host: host.into(), subset: subset.into() }
    }

    #[test]
    fn single_destination_route_compiles_to_one_row() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "translateRoutes",
            "tenant-vs-single"
        );
        let v = vs(vec![HttpRoute {
            name: "default".into(),
            r#match: RouteMatch::default(),
            route: vec![WeightedDestination { destination: dest("web", ""), weight: 100 }],
        }]);
        let table = compile(&v, Protocol::Http).unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(table[0].backend.cluster, "web");
        assert_eq!(table[0].backend.weight, 100);
    }

    #[test]
    fn weighted_split_yields_one_row_per_destination() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "buildHTTPRoute",
            "tenant-vs-split"
        );
        let v = vs(vec![HttpRoute {
            name: "canary".into(),
            r#match: RouteMatch { path_prefix: Some("/api".into()), ..Default::default() },
            route: vec![
                WeightedDestination { destination: dest("web", "v1"), weight: 80 },
                WeightedDestination { destination: dest("web", "v2"), weight: 20 },
            ],
        }]);
        let table = compile(&v, Protocol::Http).unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(table[0].backend.cluster, "web|v1");
        assert_eq!(table[0].backend.weight, 80);
        assert_eq!(table[1].backend.cluster, "web|v2");
        assert_eq!(table[1].backend.weight, 20);
    }

    #[test]
    fn weights_must_sum_to_100() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "validateWeights",
            "tenant-vs-bad-weights"
        );
        let v = vs(vec![HttpRoute {
            name: "broken".into(),
            r#match: RouteMatch::default(),
            route: vec![
                WeightedDestination { destination: dest("a", ""), weight: 30 },
                WeightedDestination { destination: dest("b", ""), weight: 30 },
            ],
        }]);
        assert!(matches!(
            compile(&v, Protocol::Http),
            Err(CompileError::BadWeightSum { actual: 60, .. })
        ));
    }

    #[test]
    fn empty_destination_list_is_rejected() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "validateRoute",
            "tenant-vs-empty"
        );
        let v = vs(vec![HttpRoute {
            name: "noop".into(),
            r#match: RouteMatch::default(),
            route: vec![],
        }]);
        assert!(matches!(compile(&v, Protocol::Http), Err(CompileError::EmptyDestinations { .. })));
    }

    #[test]
    fn vs_without_hosts_is_rejected() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "buildVirtualHost",
            "tenant-vs-no-hosts"
        );
        let mut v = vs(vec![HttpRoute {
            name: "default".into(),
            r#match: RouteMatch::default(),
            route: vec![WeightedDestination { destination: dest("web", ""), weight: 100 }],
        }]);
        v.hosts.clear();
        assert!(matches!(compile(&v, Protocol::Http), Err(CompileError::NoHosts(_))));
    }

    #[test]
    fn compiled_routes_can_drive_a_waypoint() {
        // End-to-end: compile, plug into a WaypointConfig, route a request.
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "BuildHTTPRoutesForVirtualService",
            "acme"
        );
        let v = vs(vec![HttpRoute {
            name: "v1-only".into(),
            r#match: RouteMatch { path_prefix: Some("/api".into()), ..Default::default() },
            route: vec![WeightedDestination { destination: dest("web", "v1"), weight: 100 }],
        }]);
        let table = compile(&v, Protocol::Http).unwrap();
        let cfg = crate::ambient::waypoint::WaypointConfig {
            name: "acme-wp".into(),
            namespace: "acme".into(),
            tenant: TenantId::new("acme").expect("test fixture"),
            routes: table,
        };
        let r = cfg
            .route(
                &tenant,
                &crate::ambient::waypoint::Request {
                    method: "GET".into(),
                    authority: "web".into(),
                    path: "/api/v1/users".into(),
                    protocol: Protocol::Http,
                },
            )
            .unwrap();
        assert_eq!(r.backend.cluster, "web|v1");
    }
}

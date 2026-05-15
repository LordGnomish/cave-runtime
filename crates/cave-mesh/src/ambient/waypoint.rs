// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Waypoint proxy — per-namespace L7 router.
//!
//! Mirrors `pilot/pkg/networking/core/v1alpha3/waypoint.go` plus the runtime
//! L7 dispatch that sits in front of HTTP, HTTP/2 and gRPC backends.
//!
//! A waypoint is configured with an ordered route table. For every request,
//! the first route whose `Match` matches the request wins; the request is
//! then forwarded to the route's `Backend`.

use crate::ambient::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Http,
    Http2,
    Grpc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backend {
    pub cluster: String,
    pub weight: u32,
}

/// One match clause inside a route. Mirrors the `HTTPMatchRequest` shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteMatch {
    pub method: Option<String>,
    /// Exact path match — full equality.
    pub path_exact: Option<String>,
    /// Prefix match. Both prefix and exact may be set; both must match.
    pub path_prefix: Option<String>,
    pub authority: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub name: String,
    pub r#match: RouteMatch,
    pub backend: Backend,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaypointConfig {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub authority: String,
    pub path: String,
    pub protocol: Protocol,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WaypointError {
    #[error("no route matched {method} {authority}{path}")]
    NoRoute { method: String, authority: String, path: String },
    #[error("tenant {tenant} not authorised for waypoint {waypoint}")]
    TenantDenied { tenant: TenantId, waypoint: String },
    #[error("protocol mismatch: route expects {expected:?}, request is {actual:?}")]
    ProtocolMismatch { expected: Protocol, actual: Protocol },
}

impl RouteMatch {
    pub fn matches(&self, req: &Request) -> bool {
        if let Some(m) = &self.method {
            if !req.method.eq_ignore_ascii_case(m) {
                return false;
            }
        }
        if let Some(a) = &self.authority {
            if &req.authority != a {
                return false;
            }
        }
        if let Some(p) = &self.path_exact {
            if &req.path != p {
                return false;
            }
        }
        if let Some(p) = &self.path_prefix {
            if !req.path.starts_with(p) {
                return false;
            }
        }
        true
    }
}

impl WaypointConfig {
    /// Authorise + route a single request.
    pub fn route(&self, tenant: &TenantId, req: &Request) -> Result<&Route, WaypointError> {
        if &self.tenant != tenant {
            return Err(WaypointError::TenantDenied {
                tenant: tenant.clone(),
                waypoint: self.name.clone(),
            });
        }
        for r in &self.routes {
            if r.r#match.matches(req) {
                if r.protocol != req.protocol {
                    return Err(WaypointError::ProtocolMismatch {
                        expected: r.protocol,
                        actual: req.protocol,
                    });
                }
                return Ok(r);
            }
        }
        Err(WaypointError::NoRoute {
            method: req.method.clone(),
            authority: req.authority.clone(),
            path: req.path.clone(),
        })
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::istio(
    "pilot/pkg/networking/core/v1alpha3/waypoint.go",
    "buildWaypointInbound",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn route(name: &str, m: RouteMatch, cluster: &str, proto: Protocol) -> Route {
        Route {
            name: name.into(),
            r#match: m,
            backend: Backend { cluster: cluster.into(), weight: 100 },
            protocol: proto,
        }
    }

    fn cfg(tenant: &str, routes: Vec<Route>) -> WaypointConfig {
        WaypointConfig {
            name: format!("{tenant}-wp"),
            namespace: tenant.into(),
            tenant: TenantId::new(tenant).expect("test fixture"),
            routes,
        }
    }

    fn req(method: &str, authority: &str, path: &str, proto: Protocol) -> Request {
        Request { method: method.into(), authority: authority.into(), path: path.into(), protocol: proto }
    }

    #[test]
    fn first_matching_route_wins() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/waypoint.go",
            "buildWaypointInbound",
            "acme"
        );
        let c = cfg(
            "acme",
            vec![
                route(
                    "v1",
                    RouteMatch { path_prefix: Some("/v1".into()), ..Default::default() },
                    "web-v1",
                    Protocol::Http,
                ),
                route("default", RouteMatch::default(), "web-default", Protocol::Http),
            ],
        );
        let r = c.route(&tenant, &req("GET", "web", "/v1/users", Protocol::Http)).unwrap();
        assert_eq!(r.backend.cluster, "web-v1");
    }

    #[test]
    fn route_match_method_is_case_insensitive() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "isCatchAllMatch",
            "acme"
        );
        let c = cfg(
            "acme",
            vec![route(
                "post-only",
                RouteMatch { method: Some("POST".into()), ..Default::default() },
                "writes",
                Protocol::Http,
            )],
        );
        let r = c.route(&tenant, &req("post", "web", "/", Protocol::Http)).unwrap();
        assert_eq!(r.backend.cluster, "writes");
    }

    #[test]
    fn no_match_returns_no_route_error() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/waypoint.go",
            "noMatch",
            "acme"
        );
        let c = cfg(
            "acme",
            vec![route(
                "post-only",
                RouteMatch { method: Some("POST".into()), ..Default::default() },
                "writes",
                Protocol::Http,
            )],
        );
        let err = c.route(&tenant, &req("GET", "web", "/", Protocol::Http)).unwrap_err();
        assert!(matches!(err, WaypointError::NoRoute { .. }));
    }

    #[test]
    fn cross_tenant_routing_is_refused() {
        let (_cite, attacker) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/waypoint.go",
            "tenantCheck",
            "tenant-attacker"
        );
        let c = cfg("acme", vec![route("default", RouteMatch::default(), "web", Protocol::Http)]);
        let err = c.route(&attacker, &req("GET", "web", "/", Protocol::Http)).unwrap_err();
        assert!(matches!(err, WaypointError::TenantDenied { .. }));
    }

    #[test]
    fn grpc_request_must_hit_grpc_route() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/waypoint.go",
            "buildWaypointInbound",
            "acme"
        );
        let c = cfg(
            "acme",
            vec![route(
                "default",
                RouteMatch::default(),
                "web",
                Protocol::Http,
            )],
        );
        let err = c.route(&tenant, &req("POST", "web", "/svc/Method", Protocol::Grpc)).unwrap_err();
        assert!(matches!(err, WaypointError::ProtocolMismatch { .. }));
    }

    #[test]
    fn exact_path_match_takes_priority_when_listed_first() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pilot/pkg/networking/core/v1alpha3/route/route.go",
            "translateRoute",
            "acme"
        );
        let c = cfg(
            "acme",
            vec![
                route(
                    "exact-health",
                    RouteMatch { path_exact: Some("/healthz".into()), ..Default::default() },
                    "health-svc",
                    Protocol::Http,
                ),
                route(
                    "prefix-all",
                    RouteMatch { path_prefix: Some("/".into()), ..Default::default() },
                    "default-svc",
                    Protocol::Http,
                ),
            ],
        );
        let r = c.route(&tenant, &req("GET", "web", "/healthz", Protocol::Http)).unwrap();
        assert_eq!(r.backend.cluster, "health-svc");
    }
}

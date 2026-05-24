// SPDX-License-Identifier: AGPL-3.0-or-later
//! Route compilation + dispatch.
//!
//! Pre-compiles the route table into a priority-ordered list. Match results
//! include the matched route, the resolved service, and the rewritten path
//! that the proxy layer will send upstream.

use crate::error::{AGwError, AGwResult};
use crate::matcher::{header_matches, host_matches, method_matches, path_matches, sni_matches};
use crate::models::{PathHandling, Protocol, Route, Service};
use crate::store::GwStore;
use std::sync::Arc;

/// Inputs to route matching — one HTTP request.
#[derive(Debug, Clone)]
pub struct RequestCtx {
    pub method: String,
    pub host: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub sni: Option<String>,
    pub protocol: Protocol,
    pub source_ip: Option<String>,
    pub destination_port: Option<u16>,
}

/// Successful match — what the proxy should do next.
#[derive(Debug, Clone)]
pub struct RouteMatch {
    pub route: Route,
    pub service: Option<Service>,
    pub rewritten_path: String,
    pub upstream_host_override: Option<String>,
}

#[derive(Clone)]
pub struct Router {
    store: Arc<GwStore>,
}

impl Router {
    pub fn new(store: Arc<GwStore>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<GwStore> {
        &self.store
    }

    /// Find the first route that matches the request, ordered by Kong's
    /// priority rules: regex_priority desc, then specificity (path length),
    /// then creation order (stable).
    pub fn r#match(&self, ctx: &RequestCtx) -> AGwResult<RouteMatch> {
        let mut candidates: Vec<(Route, usize)> = Vec::new();
        for route in self.store.list_routes() {
            if !route.protocols.contains(&ctx.protocol) {
                continue;
            }
            if !method_matches(&route.methods, &ctx.method) {
                continue;
            }
            if !host_matches(&route.hosts, &ctx.host) {
                continue;
            }
            if ctx.protocol.is_tls() {
                let sni = ctx.sni.as_deref().unwrap_or(&ctx.host);
                if !sni_matches(&route.snis, sni) {
                    continue;
                }
            }
            if !header_matches(&route.headers, &ctx.headers) {
                continue;
            }
            let pm = path_matches(&route.paths, &ctx.path);
            if !pm.matched {
                continue;
            }
            candidates.push((route, pm.matched_prefix_len));
        }
        candidates.sort_by(|a, b| {
            b.0.regex_priority
                .cmp(&a.0.regex_priority)
                .then(b.1.cmp(&a.1))
                .then(a.0.name.cmp(&b.0.name))
        });
        let (route, prefix_len) = candidates
            .into_iter()
            .next()
            .ok_or_else(|| AGwError::RouteNotFound(format!("{} {}", ctx.method, ctx.path)))?;
        let rewritten_path = self.rewrite_path(&route, &ctx.path, prefix_len);
        let service = match route.service_id {
            Some(id) => Some(self.store.get_service(id)?),
            None => None,
        };
        let upstream_host_override = if route.preserve_host { Some(ctx.host.clone()) } else { None };
        Ok(RouteMatch {
            route,
            service,
            rewritten_path,
            upstream_host_override,
        })
    }

    /// Apply `strip_path` + `path_handling` semantics to the URI.
    pub fn rewrite_path(&self, route: &Route, path: &str, prefix_len: usize) -> String {
        if !route.strip_path || prefix_len == 0 {
            return path.to_string();
        }
        let rest = &path[prefix_len..];
        match route.path_handling {
            PathHandling::V0 => {
                if rest.is_empty() {
                    "/".to_string()
                } else if rest.starts_with('/') {
                    rest.to_string()
                } else {
                    format!("/{rest}")
                }
            }
            PathHandling::V1 => {
                if rest.is_empty() {
                    "/".to_string()
                } else if rest.starts_with('/') {
                    rest.to_string()
                } else {
                    format!("/{rest}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Protocol, Route, Service};

    fn setup() -> (Arc<GwStore>, uuid::Uuid) {
        let s = Arc::new(GwStore::new());
        let svc_id = s.upsert_service(Service::new("svc", "backend", 80)).unwrap();
        (s, svc_id)
    }

    #[test]
    fn match_by_path_prefix() {
        let (s, svc_id) = setup();
        let mut r = Route::new("r1");
        r.paths = vec!["/api".into()];
        r.service_id = Some(svc_id);
        s.upsert_route(r).unwrap();
        let router = Router::new(s);
        let ctx = RequestCtx {
            method: "GET".into(),
            host: "h".into(),
            path: "/api/users".into(),
            headers: vec![],
            sni: None,
            protocol: Protocol::Http,
            source_ip: None,
            destination_port: None,
        };
        let m = router.r#match(&ctx).unwrap();
        assert_eq!(m.route.name, "r1");
        assert_eq!(m.rewritten_path, "/users");
        assert!(m.service.is_some());
    }

    #[test]
    fn no_match_returns_route_not_found() {
        let (s, _) = setup();
        let mut r = Route::new("r1");
        r.paths = vec!["/api".into()];
        s.upsert_route(r).unwrap();
        let router = Router::new(s);
        let ctx = RequestCtx {
            method: "GET".into(),
            host: "h".into(),
            path: "/nope".into(),
            headers: vec![],
            sni: None,
            protocol: Protocol::Http,
            source_ip: None,
            destination_port: None,
        };
        assert!(matches!(router.r#match(&ctx), Err(AGwError::RouteNotFound(_))));
    }

    #[test]
    fn regex_priority_orders_first() {
        let (s, svc_id) = setup();
        let mut low = Route::new("low");
        low.paths = vec!["/api".into()];
        low.service_id = Some(svc_id);
        low.regex_priority = 0;
        let mut high = Route::new("high");
        high.paths = vec!["/api".into()];
        high.service_id = Some(svc_id);
        high.regex_priority = 100;
        s.upsert_route(low).unwrap();
        s.upsert_route(high).unwrap();
        let router = Router::new(s);
        let ctx = RequestCtx {
            method: "GET".into(),
            host: "h".into(),
            path: "/api/x".into(),
            headers: vec![],
            sni: None,
            protocol: Protocol::Http,
            source_ip: None,
            destination_port: None,
        };
        let m = router.r#match(&ctx).unwrap();
        assert_eq!(m.route.name, "high");
    }

    #[test]
    fn strip_path_false_keeps_full_path() {
        let (s, svc_id) = setup();
        let mut r = Route::new("r1");
        r.paths = vec!["/api".into()];
        r.service_id = Some(svc_id);
        r.strip_path = false;
        s.upsert_route(r).unwrap();
        let router = Router::new(s);
        let ctx = RequestCtx {
            method: "GET".into(),
            host: "h".into(),
            path: "/api/users".into(),
            headers: vec![],
            sni: None,
            protocol: Protocol::Http,
            source_ip: None,
            destination_port: None,
        };
        let m = router.r#match(&ctx).unwrap();
        assert_eq!(m.rewritten_path, "/api/users");
    }

    #[test]
    fn preserve_host_override() {
        let (s, svc_id) = setup();
        let mut r = Route::new("r1");
        r.paths = vec!["/".into()];
        r.service_id = Some(svc_id);
        r.preserve_host = true;
        s.upsert_route(r).unwrap();
        let router = Router::new(s);
        let ctx = RequestCtx {
            method: "GET".into(),
            host: "client.example".into(),
            path: "/x".into(),
            headers: vec![],
            sni: None,
            protocol: Protocol::Http,
            source_ip: None,
            destination_port: None,
        };
        let m = router.r#match(&ctx).unwrap();
        assert_eq!(m.upstream_host_override.as_deref(), Some("client.example"));
    }
}

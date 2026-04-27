//! Cilium agent REST API — the surface the `cilium` CLI hits.
//!
//! Mirrors `api/v1/server/restapi/...` plus the route registry in
//! `daemon/cmd/api.go`. The agent exposes a Unix-socket REST API the
//! local CLI uses for `cilium endpoint list`, `cilium policy get`,
//! `cilium service list`, `cilium identity list`, `cilium bpf map list`,
//! etc.
//!
//! We model the URI taxonomy + handler dispatch — concrete handlers
//! defer to the per-subsystem managers we already have.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiRoute {
    pub method: HttpMethod,
    pub path: String, // e.g. "/v1/endpoint/{id}"
    pub handler: String, // logical handler name
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiRequest {
    pub method: HttpMethod,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApiError {
    #[error("route not found: {method:?} {path}")]
    NotFound { method: HttpMethod, path: String },
    #[error("route conflict: {method:?} {path}")]
    Conflict { method: HttpMethod, path: String },
    #[error("tenant {tenant} cannot mutate API server owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct AgentApi {
    pub tenant: TenantId,
    routes: Vec<ApiRoute>,
    /// Per-route hit counter for /metrics observability.
    hits: HashMap<String, u64>,
}

impl AgentApi {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, routes: Vec::new(), hits: HashMap::new() }
    }

    pub fn register(&mut self, route: ApiRoute) -> Result<(), ApiError> {
        if self.routes.iter().any(|r| r.method == route.method && r.path == route.path) {
            return Err(ApiError::Conflict { method: route.method, path: route.path });
        }
        self.routes.push(route);
        Ok(())
    }

    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    pub fn route(&self, method: HttpMethod, path: &str) -> Option<&ApiRoute> {
        self.routes.iter().find(|r| r.method == method && match_path(&r.path, path))
    }

    /// Resolve a request to its handler name, recording a hit and
    /// returning the matched route + extracted path params.
    pub fn dispatch(&mut self, req: &ApiRequest) -> Result<(String, Vec<(String, String)>), ApiError> {
        let route = self
            .routes
            .iter()
            .find(|r| r.method == req.method && match_path(&r.path, &req.path))
            .ok_or_else(|| ApiError::NotFound { method: req.method, path: req.path.clone() })?;
        let params = extract_params(&route.path, &req.path);
        let handler = route.handler.clone();
        *self.hits.entry(handler.clone()).or_default() += 1;
        Ok((handler, params))
    }

    pub fn hits_for(&self, handler: &str) -> u64 {
        self.hits.get(handler).copied().unwrap_or(0)
    }

    pub fn total_hits(&self) -> u64 {
        self.hits.values().sum()
    }

    /// Convenience: register the canonical Cilium agent route table.
    pub fn install_default_routes(&mut self) -> Result<(), ApiError> {
        let routes = canonical_routes();
        for r in routes {
            self.register(r)?;
        }
        Ok(())
    }
}

/// Returns the canonical Cilium agent route table.
pub fn canonical_routes() -> Vec<ApiRoute> {
    vec![
        ApiRoute { method: HttpMethod::Get, path: "/v1/endpoint".into(), handler: "GetEndpoint".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/endpoint/{id}".into(), handler: "GetEndpointID".into() },
        ApiRoute { method: HttpMethod::Put, path: "/v1/endpoint/{id}".into(), handler: "PutEndpointID".into() },
        ApiRoute { method: HttpMethod::Delete, path: "/v1/endpoint/{id}".into(), handler: "DeleteEndpointID".into() },
        ApiRoute { method: HttpMethod::Patch, path: "/v1/endpoint/{id}/labels".into(), handler: "PatchEndpointIDLabels".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/policy".into(), handler: "GetPolicy".into() },
        ApiRoute { method: HttpMethod::Put, path: "/v1/policy".into(), handler: "PutPolicy".into() },
        ApiRoute { method: HttpMethod::Delete, path: "/v1/policy".into(), handler: "DeletePolicy".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/service".into(), handler: "GetService".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/service/{id}".into(), handler: "GetServiceID".into() },
        ApiRoute { method: HttpMethod::Put, path: "/v1/service/{id}".into(), handler: "PutServiceID".into() },
        ApiRoute { method: HttpMethod::Delete, path: "/v1/service/{id}".into(), handler: "DeleteServiceID".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/identity".into(), handler: "GetIdentity".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/identity/{id}".into(), handler: "GetIdentityID".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/healthz".into(), handler: "GetHealthz".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/config".into(), handler: "GetConfig".into() },
        ApiRoute { method: HttpMethod::Patch, path: "/v1/config".into(), handler: "PatchConfig".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/metrics".into(), handler: "GetMetrics".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/map".into(), handler: "GetMap".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/map/{name}".into(), handler: "GetMapName".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/cluster/nodes".into(), handler: "GetClusterNodes".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/fqdn/cache".into(), handler: "GetFqdnCache".into() },
        ApiRoute { method: HttpMethod::Get, path: "/v1/ipam".into(), handler: "GetIpam".into() },
        ApiRoute { method: HttpMethod::Post, path: "/v1/ipam".into(), handler: "PostIpam".into() },
        ApiRoute { method: HttpMethod::Delete, path: "/v1/ipam/{ip}".into(), handler: "DeleteIpamIP".into() },
    ]
}

/// True if `pattern` like `/v1/endpoint/{id}` matches `path` like
/// `/v1/endpoint/42`.
fn match_path(pattern: &str, path: &str) -> bool {
    let p = pattern.trim_matches('/');
    let q = path.trim_matches('/');
    let pp: Vec<&str> = p.split('/').collect();
    let qq: Vec<&str> = q.split('/').collect();
    if pp.len() != qq.len() {
        return false;
    }
    for (a, b) in pp.iter().zip(qq.iter()) {
        if a.starts_with('{') && a.ends_with('}') {
            continue;
        }
        if a != b {
            return false;
        }
    }
    true
}

fn extract_params(pattern: &str, path: &str) -> Vec<(String, String)> {
    let p = pattern.trim_matches('/');
    let q = path.trim_matches('/');
    let pp: Vec<&str> = p.split('/').collect();
    let qq: Vec<&str> = q.split('/').collect();
    let mut out = Vec::new();
    for (a, b) in pp.iter().zip(qq.iter()) {
        if a.starts_with('{') && a.ends_with('}') {
            let key = &a[1..a.len() - 1];
            out.push((key.to_string(), (*b).to_string()));
        }
    }
    out
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("daemon/cmd/api.go", "Daemon");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn api(tenant: TenantId) -> AgentApi {
        AgentApi::new(tenant)
    }

    fn req(method: HttpMethod, path: &str) -> ApiRequest {
        ApiRequest { method, path: path.into(), query: vec![], body: vec![] }
    }

    // ── Path matching ───────────────────────────────────────────────────────

    #[test]
    fn path_match_exact() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "Path.Exact", "tenant-api-pe");
        assert!(match_path("/v1/policy", "/v1/policy"));
        assert!(!match_path("/v1/policy", "/v1/policy/x"));
    }

    #[test]
    fn path_match_with_param_placeholder() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "Path.Param", "tenant-api-pp");
        assert!(match_path("/v1/endpoint/{id}", "/v1/endpoint/42"));
        assert!(!match_path("/v1/endpoint/{id}", "/v1/endpoint"));
        assert!(!match_path("/v1/endpoint/{id}", "/v1/endpoint/42/extra"));
    }

    #[test]
    fn extract_params_returns_key_value_pairs() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "Path.ExtractParams", "tenant-api-ep");
        let p = extract_params("/v1/endpoint/{id}", "/v1/endpoint/42");
        assert_eq!(p, vec![("id".to_string(), "42".to_string())]);
    }

    #[test]
    fn extract_params_multi_segment() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "Path.MultiParam", "tenant-api-mp");
        let p = extract_params("/v1/endpoint/{id}/labels/{label}", "/v1/endpoint/42/labels/app");
        assert!(p.contains(&("id".to_string(), "42".to_string())));
        assert!(p.contains(&("label".to_string(), "app".to_string())));
    }

    // ── Register / dispatch ────────────────────────────────────────────────

    #[test]
    fn register_and_route_lookup() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Register", "tenant-api-r");
        let mut a = api(tenant);
        a.register(ApiRoute { method: HttpMethod::Get, path: "/v1/policy".into(), handler: "GetPolicy".into() }).unwrap();
        let r = a.route(HttpMethod::Get, "/v1/policy").unwrap();
        assert_eq!(r.handler, "GetPolicy");
    }

    #[test]
    fn register_duplicate_rejected() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Register.Conflict", "tenant-api-rc");
        let mut a = api(tenant);
        a.register(ApiRoute { method: HttpMethod::Get, path: "/v1/policy".into(), handler: "GetPolicy".into() }).unwrap();
        let err = a.register(ApiRoute { method: HttpMethod::Get, path: "/v1/policy".into(), handler: "Other".into() }).unwrap_err();
        assert!(matches!(err, ApiError::Conflict { .. }));
    }

    #[test]
    fn register_same_path_different_method_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Register.MethodDistinct", "tenant-api-rd");
        let mut a = api(tenant);
        a.register(ApiRoute { method: HttpMethod::Get, path: "/v1/policy".into(), handler: "GetPolicy".into() }).unwrap();
        a.register(ApiRoute { method: HttpMethod::Put, path: "/v1/policy".into(), handler: "PutPolicy".into() }).unwrap();
        assert_eq!(a.route_count(), 2);
    }

    // ── Dispatch ────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_returns_handler_for_match() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Dispatch", "tenant-api-d");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        let (h, _) = a.dispatch(&req(HttpMethod::Get, "/v1/policy")).unwrap();
        assert_eq!(h, "GetPolicy");
    }

    #[test]
    fn dispatch_extracts_params() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Dispatch.Params", "tenant-api-dp");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        let (h, params) = a.dispatch(&req(HttpMethod::Get, "/v1/endpoint/42")).unwrap();
        assert_eq!(h, "GetEndpointID");
        assert_eq!(params, vec![("id".to_string(), "42".to_string())]);
    }

    #[test]
    fn dispatch_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Dispatch.NotFound", "tenant-api-dnf");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        let err = a.dispatch(&req(HttpMethod::Get, "/v1/garbage")).unwrap_err();
        assert!(matches!(err, ApiError::NotFound { .. }));
    }

    #[test]
    fn dispatch_wrong_method_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Dispatch.WrongMethod", "tenant-api-dwm");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        // /v1/identity is GET-only.
        let err = a.dispatch(&req(HttpMethod::Delete, "/v1/identity")).unwrap_err();
        assert!(matches!(err, ApiError::NotFound { .. }));
    }

    // ── Hit counters ───────────────────────────────────────────────────────

    #[test]
    fn dispatch_increments_hit_counter() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Hits", "tenant-api-h");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        a.dispatch(&req(HttpMethod::Get, "/v1/policy")).unwrap();
        a.dispatch(&req(HttpMethod::Get, "/v1/policy")).unwrap();
        a.dispatch(&req(HttpMethod::Get, "/v1/healthz")).unwrap();
        assert_eq!(a.hits_for("GetPolicy"), 2);
        assert_eq!(a.hits_for("GetHealthz"), 1);
        assert_eq!(a.total_hits(), 3);
    }

    #[test]
    fn hits_for_unknown_handler_returns_zero() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Hits.Unknown", "tenant-api-hnf");
        let a = api(tenant);
        assert_eq!(a.hits_for("ghost"), 0);
    }

    // ── Default routes ─────────────────────────────────────────────────────

    #[test]
    fn install_default_routes_registers_canonical_set() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "InstallDefault", "tenant-api-id");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        // Spot-check a few canonical handlers.
        assert!(a.route(HttpMethod::Get, "/v1/endpoint").is_some());
        assert!(a.route(HttpMethod::Get, "/v1/policy").is_some());
        assert!(a.route(HttpMethod::Get, "/v1/healthz").is_some());
        assert!(a.route(HttpMethod::Get, "/v1/identity/256").is_some());
        assert!(a.route(HttpMethod::Get, "/v1/map/cilium_ipcache").is_some());
        assert!(a.route(HttpMethod::Get, "/v1/cluster/nodes").is_some());
        assert!(a.route(HttpMethod::Get, "/v1/fqdn/cache").is_some());
    }

    #[test]
    fn canonical_routes_count_is_reasonable() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "CanonicalCount", "tenant-api-cc");
        let r = canonical_routes();
        assert!(r.len() >= 20);
    }

    #[test]
    fn install_default_routes_is_idempotent_failure_returns_conflict() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "InstallDefault.Idempotent", "tenant-api-iid");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        let err = a.install_default_routes().unwrap_err();
        assert!(matches!(err, ApiError::Conflict { .. }));
    }

    // ── Method coverage ────────────────────────────────────────────────────

    #[test]
    fn endpoint_id_supports_get_put_delete() {
        let (_c, tenant) = cilium_test_ctx!("daemon/cmd/api.go", "Endpoint.MethodCoverage", "tenant-api-emc");
        let mut a = api(tenant);
        a.install_default_routes().unwrap();
        for m in [HttpMethod::Get, HttpMethod::Put, HttpMethod::Delete] {
            assert!(a.route(m, "/v1/endpoint/42").is_some());
        }
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn api_route_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "Route.Serde", "tenant-api-rserde");
        let r = ApiRoute { method: HttpMethod::Patch, path: "/v1/endpoint/{id}/labels".into(), handler: "PatchEndpointIDLabels".into() };
        let s = serde_json::to_string(&r).unwrap();
        let back: ApiRoute = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn http_method_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "Method.Serde", "tenant-api-mserde");
        for m in [HttpMethod::Get, HttpMethod::Post, HttpMethod::Put, HttpMethod::Patch, HttpMethod::Delete] {
            let s = serde_json::to_string(&m).unwrap();
            let back: HttpMethod = serde_json::from_str(&s).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn api_request_response_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("daemon/cmd/api.go", "ReqResp.Serde", "tenant-api-rrserde");
        let r = req(HttpMethod::Get, "/v1/endpoint/42");
        let s = serde_json::to_string(&r).unwrap();
        let back: ApiRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }
}

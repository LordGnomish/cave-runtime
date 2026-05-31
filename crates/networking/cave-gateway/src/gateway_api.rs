// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubernetes Gateway API translation.
//!
//! Translates `gateway.networking.k8s.io/v1` `HTTPRoute` resources into the
//! gateway's internal Kong-style [`Route`] / [`Service`] model. This lets
//! cave-gateway be driven declaratively by Gateway API CRDs in addition to the
//! Kong admin API.
//!
//! The headline behaviour is the Gateway API **conflict-resolution
//! precedence** (`apis/v1/httproute_types.go`): when several matches could
//! serve a request the most specific wins, ordered by
//!   1. `Exact` path match
//!   2. longest `PathPrefix` match (by characters)
//!   3. method match present
//!   4. largest number of header matches
//!   5. largest number of query-param matches
//!
//! Only the data-plane subset is translated here (matches / filters /
//! backendRefs). Cross-resource status reporting and Gateway/GatewayClass
//! admission live in the k8s control loop (cave-controller-manager).

use crate::models::{Protocol, Route, Service};
use serde::{Deserialize, Serialize};

// ── Gateway API resource model (v1) ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpRoute {
    #[serde(default)]
    pub hostnames: Vec<String>,
    #[serde(default)]
    pub rules: Vec<HttpRouteRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpRouteRule {
    #[serde(default)]
    pub matches: Vec<HttpRouteMatch>,
    #[serde(default)]
    pub filters: Vec<HttpRouteFilter>,
    #[serde(default, rename = "backendRefs")]
    pub backend_refs: Vec<BackendRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpRouteMatch {
    pub path: Option<PathMatch>,
    #[serde(default)]
    pub headers: Vec<HeaderMatch>,
    pub method: Option<String>,
    #[serde(default, rename = "queryParams")]
    pub query_params: Vec<QueryParamMatch>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathMatchType {
    Exact,
    PathPrefix,
    RegularExpression,
}

impl Default for PathMatchType {
    fn default() -> Self {
        PathMatchType::PathPrefix
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMatch {
    #[serde(default, rename = "type")]
    pub match_type: PathMatchType,
    #[serde(default = "default_path_value")]
    pub value: String,
}

fn default_path_value() -> String {
    "/".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderMatch {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParamMatch {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HttpRouteFilter {
    RequestHeaderModifier {
        #[serde(default)]
        set: Vec<HeaderKv>,
        #[serde(default)]
        add: Vec<HeaderKv>,
        #[serde(default)]
        remove: Vec<String>,
    },
    RequestRedirect {
        scheme: Option<String>,
        hostname: Option<String>,
        port: Option<u16>,
        #[serde(rename = "statusCode")]
        status_code: Option<u16>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderKv {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendRef {
    pub name: String,
    pub port: Option<u16>,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

// ── Precedence ───────────────────────────────────────────────────────────────

/// Sortable precedence key — larger compares as **more specific**.
///
/// Tuple order mirrors the Gateway API conflict-resolution list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PrecedenceKey {
    pub path_rank: u8,
    pub path_len: usize,
    pub method: u8,
    pub header_count: usize,
    pub query_count: usize,
}

/// Compute the precedence key for a single match.
pub fn precedence_key(m: &HttpRouteMatch) -> PrecedenceKey {
    let (path_rank, path_len) = match &m.path {
        Some(p) => match p.match_type {
            // Exact wins outright; RegularExpression is below PathPrefix.
            PathMatchType::Exact => (3, p.value.len()),
            PathMatchType::PathPrefix => (2, p.value.len()),
            PathMatchType::RegularExpression => (1, p.value.len()),
        },
        None => (0, 0),
    };
    PrecedenceKey {
        path_rank,
        path_len,
        method: if m.method.is_some() { 1 } else { 0 },
        header_count: m.headers.len(),
        query_count: m.query_params.len(),
    }
}

// ── Translation ──────────────────────────────────────────────────────────────

/// A single translated route: the internal [`Route`] plus the backend
/// [`Service`]s it forwards to, in precedence order.
#[derive(Debug, Clone, Serialize)]
pub struct TranslatedRoute {
    pub route: Route,
    pub services: Vec<Service>,
    pub precedence: PrecedenceKey,
}

/// Translate one match into an internal [`Route`] (no backends attached).
pub fn translate_match(m: &HttpRouteMatch, hostnames: &[String]) -> Route {
    let mut route = Route::new(uuid::Uuid::nil());

    if let Some(p) = &m.path {
        route.paths = Some(vec![p.value.clone()]);
        // Regex matches drive the matcher's regex engine; flag with a positive
        // priority so they are evaluated as regexes (not literal prefixes).
        if p.match_type == PathMatchType::RegularExpression {
            route.regex_priority = 1;
        }
    }
    if let Some(method) = &m.method {
        route.methods = Some(vec![method.clone()]);
    }
    if !hostnames.is_empty() {
        route.hosts = Some(hostnames.to_vec());
    }
    if !m.headers.is_empty() {
        let mut hdrs = std::collections::HashMap::new();
        for h in &m.headers {
            hdrs.entry(h.name.clone())
                .or_insert_with(Vec::new)
                .push(h.value.clone());
        }
        route.headers = Some(hdrs);
    }
    route
}

/// Translate a backend ref into an internal [`Service`].
pub fn translate_backend(b: &BackendRef) -> Service {
    // Gateway API backendRefs default to port 80 for HTTP when unspecified.
    let mut svc = Service::new(b.name.clone(), b.port.unwrap_or(80), Protocol::Http);
    svc.name = Some(b.name.clone());
    svc
}

/// Translate an entire HTTPRoute into precedence-ordered internal routes.
///
/// One [`TranslatedRoute`] is produced per `(rule × match)`; rules with no
/// explicit match default to a catch-all `PathPrefix /`. The result is sorted
/// most-specific-first per the Gateway API conflict-resolution rules.
pub fn translate(route: &HttpRoute) -> Vec<TranslatedRoute> {
    let mut out = Vec::new();
    for rule in &route.rules {
        let services: Vec<Service> = rule.backend_refs.iter().map(translate_backend).collect();
        // A rule with no matches is a catch-all (PathPrefix "/").
        let matches: Vec<HttpRouteMatch> = if rule.matches.is_empty() {
            vec![HttpRouteMatch::default()]
        } else {
            rule.matches.clone()
        };
        for m in &matches {
            out.push(TranslatedRoute {
                route: translate_match(m, &route.hostnames),
                services: services.clone(),
                precedence: precedence_key(m),
            });
        }
    }
    // Most specific first (descending precedence). Stable sort preserves
    // list order for ties, matching "first matching rule in list order".
    out.sort_by(|a, b| b.precedence.cmp(&a.precedence));
    out
}

// ── HTTP surface ─────────────────────────────────────────────────────────────

use axum::{Json, Router, routing::post};

/// `POST /admin/v1/gateway-api/httproutes/translate` — dry-run translation of a
/// Gateway API HTTPRoute into the precedence-ordered internal route/service
/// model. Returns the same data the controller loop would apply to the store.
async fn translate_handler(Json(route): Json<HttpRoute>) -> Json<serde_json::Value> {
    let translated = translate(&route);
    Json(serde_json::json!({
        "count": translated.len(),
        "routes": translated,
    }))
}

/// Router for the Gateway API translation surface.
pub fn router() -> Router {
    Router::new().route("/httproutes/translate", post(translate_handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "hostnames": ["shop.example.com"],
        "rules": [{
            "matches": [{
                "path": {"type": "PathPrefix", "value": "/api"},
                "method": "GET",
                "headers": [{"name": "x-env", "value": "prod"}]
            }],
            "filters": [{"type": "RequestRedirect", "scheme": "https"}],
            "backendRefs": [
                {"name": "api-svc", "port": 8080, "weight": 3},
                {"name": "api-canary", "port": 8080, "weight": 1}
            ]
        }]
    }"#;

    fn pm(t: PathMatchType, v: &str) -> PathMatch {
        PathMatch { match_type: t, value: v.into() }
    }

    #[test]
    fn parses_httproute_json() {
        let r: HttpRoute = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(r.hostnames, vec!["shop.example.com"]);
        assert_eq!(r.rules.len(), 1);
        let rule = &r.rules[0];
        assert_eq!(rule.matches[0].method.as_deref(), Some("GET"));
        assert_eq!(rule.matches[0].path.as_ref().unwrap().value, "/api");
        assert_eq!(rule.backend_refs.len(), 2);
        assert_eq!(rule.backend_refs[0].weight, 3);
        assert_eq!(rule.backend_refs[1].weight, 1);
        assert_eq!(rule.filters.len(), 1);
    }

    #[test]
    fn backend_ref_weight_defaults_to_one() {
        let b: BackendRef = serde_json::from_str(r#"{"name":"x","port":80}"#).unwrap();
        assert_eq!(b.weight, 1);
    }

    #[test]
    fn precedence_exact_beats_prefix() {
        let exact = HttpRouteMatch { path: Some(pm(PathMatchType::Exact, "/a")), ..Default::default() };
        let prefix = HttpRouteMatch { path: Some(pm(PathMatchType::PathPrefix, "/a/very/long")), ..Default::default() };
        assert!(precedence_key(&exact) > precedence_key(&prefix));
    }

    #[test]
    fn precedence_longer_prefix_wins() {
        let short = HttpRouteMatch { path: Some(pm(PathMatchType::PathPrefix, "/a")), ..Default::default() };
        let long = HttpRouteMatch { path: Some(pm(PathMatchType::PathPrefix, "/a/b/c")), ..Default::default() };
        assert!(precedence_key(&long) > precedence_key(&short));
    }

    #[test]
    fn precedence_method_then_headers_then_query() {
        let base = HttpRouteMatch { path: Some(pm(PathMatchType::PathPrefix, "/a")), ..Default::default() };
        let with_method = HttpRouteMatch { method: Some("GET".into()), ..base.clone() };
        assert!(precedence_key(&with_method) > precedence_key(&base));

        let with_headers = HttpRouteMatch {
            method: Some("GET".into()),
            headers: vec![HeaderMatch { name: "a".into(), value: "1".into() }],
            ..base.clone()
        };
        assert!(precedence_key(&with_headers) > precedence_key(&with_method));

        let with_query = HttpRouteMatch {
            method: Some("GET".into()),
            headers: vec![HeaderMatch { name: "a".into(), value: "1".into() }],
            query_params: vec![QueryParamMatch { name: "q".into(), value: "1".into() }],
            ..base
        };
        assert!(precedence_key(&with_query) > precedence_key(&with_headers));
    }

    #[test]
    fn precedence_regex_ranks_below_prefix() {
        let regex = HttpRouteMatch { path: Some(pm(PathMatchType::RegularExpression, "/a/.*")), ..Default::default() };
        let prefix = HttpRouteMatch { path: Some(pm(PathMatchType::PathPrefix, "/a")), ..Default::default() };
        assert!(precedence_key(&prefix) > precedence_key(&regex));
    }

    #[test]
    fn translate_match_sets_path_method_host_headers() {
        let m = HttpRouteMatch {
            path: Some(pm(PathMatchType::PathPrefix, "/api")),
            method: Some("POST".into()),
            headers: vec![HeaderMatch { name: "x-env".into(), value: "prod".into() }],
            ..Default::default()
        };
        let route = translate_match(&m, &["shop.example.com".into()]);
        assert_eq!(route.paths.as_ref().unwrap(), &vec!["/api".to_string()]);
        assert_eq!(route.methods.as_ref().unwrap(), &vec!["POST".to_string()]);
        assert_eq!(route.hosts.as_ref().unwrap(), &vec!["shop.example.com".to_string()]);
        let hdrs = route.headers.as_ref().unwrap();
        assert_eq!(hdrs.get("x-env").unwrap(), &vec!["prod".to_string()]);
    }

    #[test]
    fn translate_match_regex_gets_priority() {
        let m = HttpRouteMatch { path: Some(pm(PathMatchType::RegularExpression, "/x/[0-9]+")), ..Default::default() };
        let route = translate_match(&m, &[]);
        // Regex routes are flagged via a non-zero regex_priority for the matcher.
        assert!(route.regex_priority > 0);
        assert_eq!(route.paths.as_ref().unwrap(), &vec!["/x/[0-9]+".to_string()]);
    }

    #[test]
    fn translate_backend_builds_service() {
        let b = BackendRef { name: "orders".into(), port: Some(9000), weight: 5 };
        let svc = translate_backend(&b);
        assert_eq!(svc.host, "orders");
        assert_eq!(svc.port, 9000);
        assert_eq!(svc.name.as_deref(), Some("orders"));
    }

    #[test]
    fn translate_backend_defaults_port_to_80() {
        let b = BackendRef { name: "orders".into(), port: None, weight: 1 };
        let svc = translate_backend(&b);
        assert_eq!(svc.port, 80);
    }

    #[test]
    fn translate_full_route_is_precedence_ordered() {
        let route: HttpRoute = serde_json::from_str(r#"{
            "hostnames": ["h.example.com"],
            "rules": [
                {"matches": [{"path": {"type": "PathPrefix", "value": "/a"}}], "backendRefs": [{"name":"s1","port":80}]},
                {"matches": [{"path": {"type": "Exact", "value": "/a/exact"}}], "backendRefs": [{"name":"s2","port":80}]}
            ]
        }"#).unwrap();
        let translated = translate(&route);
        assert_eq!(translated.len(), 2);
        // Exact match must be first (highest precedence).
        assert_eq!(translated[0].route.paths.as_ref().unwrap(), &vec!["/a/exact".to_string()]);
        assert_eq!(translated[0].services[0].host, "s2");
        assert!(translated[0].precedence > translated[1].precedence);
    }

    #[test]
    fn translate_attaches_all_backends() {
        let route: HttpRoute = serde_json::from_str(SAMPLE).unwrap();
        let translated = translate(&route);
        assert_eq!(translated.len(), 1);
        assert_eq!(translated[0].services.len(), 2);
        assert_eq!(translated[0].services[0].host, "api-svc");
    }
}

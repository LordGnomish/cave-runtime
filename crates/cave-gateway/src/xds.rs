//! Envoy xDS API — LDS, RDS, CDS, EDS.
//!
//! Implements the v3 xDS REST/JSON variants (not gRPC streaming).
//! Mounted at /xds/v3/.
//!
//! References:
//!   - LDS: Listener Discovery Service
//!   - RDS: Route Discovery Service
//!   - CDS: Cluster Discovery Service
//!   - EDS: Endpoint Discovery Service

use crate::models::{LbAlgorithm, Protocol, Service, Target, Upstream};
use crate::store::SharedStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn xds_router(store: SharedStore) -> Router {
    Router::new()
        // Discovery endpoints (REST/JSON)
        .route("/xds/v3/discovery/listeners", post(lds_handler))
        .route("/xds/v3/discovery/routes", post(rds_handler))
        .route("/xds/v3/discovery/clusters", post(cds_handler))
        .route("/xds/v3/discovery/endpoints", post(eds_handler))
        // Individual resource queries
        .route("/xds/v3/listener/{name}", get(get_listener))
        .route("/xds/v3/route_configuration/{name}", get(get_route_config))
        .route("/xds/v3/cluster/{name}", get(get_cluster))
        .route("/xds/v3/cluster_load_assignment/{cluster_name}", get(get_endpoints))
        .with_state(store)
}

// ── xDS request/response types ────────────────────────────────────────────────

#[derive(Deserialize)]
struct DiscoveryRequest {
    #[serde(default)]
    resource_names: Option<Vec<String>>,
    #[serde(flatten)]
    _extra: serde_json::Map<String, Value>,
}

fn version_info() -> String {
    chrono::Utc::now().timestamp().to_string()
}

fn nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ── LDS — Listener Discovery Service ─────────────────────────────────────────

/// Build an Envoy Listener from our gateway config.
fn build_listener(name: &str, port: u32, route_config_name: &str) -> Value {
    json!({
        "@type": "type.googleapis.com/envoy.config.listener.v3.Listener",
        "name": name,
        "address": {
            "socket_address": {
                "protocol": "TCP",
                "address": "0.0.0.0",
                "port_value": port
            }
        },
        "filter_chains": [{
            "filters": [{
                "name": "envoy.filters.network.http_connection_manager",
                "typed_config": {
                    "@type": "type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager",
                    "stat_prefix": "ingress_http",
                    "codec_type": "AUTO",
                    "route_config_name": route_config_name,
                    "use_remote_address": true,
                    "http_filters": [
                        {"name": "envoy.filters.http.router", "typed_config": {"@type": "type.googleapis.com/envoy.extensions.filters.http.router.v3.Router"}}
                    ]
                }
            }]
        }]
    })
}

async fn lds_handler(
    State(store): State<SharedStore>,
    Json(req): Json<DiscoveryRequest>,
) -> Json<Value> {
    let listeners = vec![
        build_listener("http", 8080, "http_route"),
        build_listener("https", 8443, "https_route"),
    ];

    Json(json!({
        "version_info": version_info(),
        "resources": listeners,
        "type_url": "type.googleapis.com/envoy.config.listener.v3.Listener",
        "nonce": nonce(),
    }))
}

async fn get_listener(
    State(store): State<SharedStore>,
    Path(name): Path<String>,
) -> Json<Value> {
    Json(build_listener(&name, 8080, &format!("{}_route", name)))
}

// ── RDS — Route Discovery Service ────────────────────────────────────────────

fn build_virtual_host(name: &str, domains: &[&str], service_name: &str, prefix: &str) -> Value {
    json!({
        "name": name,
        "domains": domains,
        "routes": [{
            "match": {"prefix": prefix},
            "route": {
                "cluster": service_name,
                "timeout": "60s",
                "retry_policy": {
                    "retry_on": "5xx",
                    "num_retries": 3,
                    "per_try_timeout": "10s"
                }
            }
        }]
    })
}

async fn rds_handler(
    State(store): State<SharedStore>,
    Json(req): Json<DiscoveryRequest>,
) -> Json<Value> {
    let requested_names = req.resource_names.unwrap_or_default();
    let routes = store.list_routes();
    let services = store.list_services();

    let mut virtual_hosts: Vec<Value> = Vec::new();

    for route in &routes {
        let service_name = route.service_id
            .and_then(|id| services.iter().find(|s| s.id == id))
            .and_then(|s| s.name.as_deref())
            .unwrap_or("default")
            .to_string();

        let domains: Vec<&str> = route.hosts
            .as_ref()
            .map(|h| h.iter().map(|s| s.as_str()).collect())
            .unwrap_or_else(|| vec!["*"]);

        let prefix = route.paths
            .as_ref()
            .and_then(|p| p.first())
            .map(|p| p.as_str())
            .unwrap_or("/");

        let vh_name = route.name.as_deref().unwrap_or(&route.id.to_string()).to_string();
        virtual_hosts.push(build_virtual_host(&vh_name, &domains, &service_name, prefix));
    }

    if virtual_hosts.is_empty() {
        virtual_hosts.push(build_virtual_host("default", &["*"], "default_cluster", "/"));
    }

    let route_configs = vec![json!({
        "@type": "type.googleapis.com/envoy.config.route.v3.RouteConfiguration",
        "name": "http_route",
        "virtual_hosts": virtual_hosts,
    })];

    Json(json!({
        "version_info": version_info(),
        "resources": route_configs,
        "type_url": "type.googleapis.com/envoy.config.route.v3.RouteConfiguration",
        "nonce": nonce(),
    }))
}

async fn get_route_config(
    State(store): State<SharedStore>,
    Path(name): Path<String>,
) -> Json<Value> {
    Json(json!({
        "@type": "type.googleapis.com/envoy.config.route.v3.RouteConfiguration",
        "name": name,
        "virtual_hosts": [{
            "name": "default",
            "domains": ["*"],
            "routes": [{"match": {"prefix": "/"}, "route": {"cluster": "default_cluster"}}]
        }]
    }))
}

// ── CDS — Cluster Discovery Service ──────────────────────────────────────────

fn lb_policy_name(algo: &LbAlgorithm) -> &'static str {
    match algo {
        LbAlgorithm::RoundRobin => "ROUND_ROBIN",
        LbAlgorithm::LeastConnections => "LEAST_REQUEST",
        LbAlgorithm::ConsistentHashing => "RING_HASH",
        LbAlgorithm::LatencyAware => "LEAST_REQUEST",
    }
}

fn build_cluster(upstream: &Upstream, targets: &[Target]) -> Value {
    let endpoints: Vec<Value> = targets.iter().map(|t| {
        let (host, port) = t.host_port();
        json!({
            "endpoint": {
                "address": {
                    "socket_address": {
                        "address": host,
                        "port_value": port
                    }
                }
            },
            "load_balancing_weight": t.weight
        })
    }).collect();

    json!({
        "@type": "type.googleapis.com/envoy.config.cluster.v3.Cluster",
        "name": upstream.name,
        "type": "EDS",
        "eds_cluster_config": {
            "eds_config": {
                "api_config_source": {
                    "api_type": "REST",
                    "cluster_names": ["xds_cluster"],
                    "refresh_delay": "5s"
                }
            },
            "service_name": upstream.name
        },
        "lb_policy": lb_policy_name(&upstream.algorithm),
        "connect_timeout": "5s",
        "health_checks": [],
        "circuit_breakers": {
            "thresholds": [{
                "priority": "DEFAULT",
                "max_connections": 1024,
                "max_pending_requests": 1024,
                "max_requests": 1024,
                "max_retries": 3
            }]
        }
    })
}

async fn cds_handler(
    State(store): State<SharedStore>,
    Json(req): Json<DiscoveryRequest>,
) -> Json<Value> {
    let requested = req.resource_names.unwrap_or_default();
    let upstreams = store.list_upstreams();

    let clusters: Vec<Value> = upstreams
        .iter()
        .filter(|u| requested.is_empty() || requested.contains(&u.name))
        .map(|u| {
            let targets = store.targets_for_upstream(&u.id);
            build_cluster(u, &targets)
        })
        .collect();

    // Also include services as clusters (direct proxy mode)
    let services = store.list_services();
    let service_clusters: Vec<Value> = services
        .iter()
        .filter(|s| s.name.is_some())
        .filter(|s| {
            let name = s.name.as_deref().unwrap_or("");
            requested.is_empty() || requested.contains(&name.to_string())
        })
        .map(|s| {
            let name = s.name.as_deref().unwrap_or("unknown");
            json!({
                "@type": "type.googleapis.com/envoy.config.cluster.v3.Cluster",
                "name": name,
                "type": "LOGICAL_DNS",
                "dns_lookup_family": "V4_ONLY",
                "load_assignment": {
                    "cluster_name": name,
                    "endpoints": [{
                        "lb_endpoints": [{
                            "endpoint": {
                                "address": {
                                    "socket_address": {
                                        "address": s.host,
                                        "port_value": s.port
                                    }
                                }
                            }
                        }]
                    }]
                },
                "lb_policy": "ROUND_ROBIN",
                "connect_timeout": format!("{}ms", s.connect_timeout),
            })
        })
        .collect();

    let all_clusters: Vec<Value> = clusters.into_iter().chain(service_clusters).collect();

    Json(json!({
        "version_info": version_info(),
        "resources": all_clusters,
        "type_url": "type.googleapis.com/envoy.config.cluster.v3.Cluster",
        "nonce": nonce(),
    }))
}

async fn get_cluster(
    State(store): State<SharedStore>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Some(upstream) = store.get_upstream_by_id_or_name(&name) {
        let targets = store.targets_for_upstream(&upstream.id);
        return Json(build_cluster(&upstream, &targets)).into_response();
    }
    if let Some(service) = store.get_service_by_id_or_name(&name) {
        return Json(json!({
            "@type": "type.googleapis.com/envoy.config.cluster.v3.Cluster",
            "name": name,
            "type": "LOGICAL_DNS",
            "load_assignment": {
                "cluster_name": name,
                "endpoints": [{"lb_endpoints": [{
                    "endpoint": {"address": {"socket_address": {"address": service.host, "port_value": service.port}}}
                }]}]
            }
        })).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

// ── EDS — Endpoint Discovery Service ─────────────────────────────────────────

fn build_cluster_load_assignment(upstream_name: &str, targets: &[Target]) -> Value {
    let lb_endpoints: Vec<Value> = targets.iter().map(|t| {
        let (host, port) = t.host_port();
        json!({
            "endpoint": {
                "address": {
                    "socket_address": {
                        "address": host,
                        "port_value": port
                    }
                }
            },
            "health_status": "HEALTHY",
            "load_balancing_weight": t.weight,
        })
    }).collect();

    json!({
        "@type": "type.googleapis.com/envoy.config.endpoint.v3.ClusterLoadAssignment",
        "cluster_name": upstream_name,
        "endpoints": [{
            "locality": {"region": "default", "zone": "default"},
            "lb_endpoints": lb_endpoints,
            "load_balancing_weight": 1
        }],
        "policy": {
            "drop_overloads": [],
            "overprovisioning_factor": 140
        }
    })
}

async fn eds_handler(
    State(store): State<SharedStore>,
    Json(req): Json<DiscoveryRequest>,
) -> Json<Value> {
    let requested = req.resource_names.unwrap_or_default();
    let upstreams = store.list_upstreams();

    let assignments: Vec<Value> = upstreams
        .iter()
        .filter(|u| requested.is_empty() || requested.contains(&u.name))
        .map(|u| {
            let targets = store.targets_for_upstream(&u.id);
            build_cluster_load_assignment(&u.name, &targets)
        })
        .collect();

    Json(json!({
        "version_info": version_info(),
        "resources": assignments,
        "type_url": "type.googleapis.com/envoy.config.endpoint.v3.ClusterLoadAssignment",
        "nonce": nonce(),
    }))
}

async fn get_endpoints(
    State(store): State<SharedStore>,
    Path(cluster_name): Path<String>,
) -> impl IntoResponse {
    match store.get_upstream_by_id_or_name(&cluster_name) {
        Some(upstream) => {
            let targets = store.targets_for_upstream(&upstream.id);
            Json(build_cluster_load_assignment(&cluster_name, &targets)).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{LbAlgorithm, Upstream};

    #[test]
    fn lb_policy_mapping() {
        assert_eq!(lb_policy_name(&LbAlgorithm::RoundRobin), "ROUND_ROBIN");
        assert_eq!(lb_policy_name(&LbAlgorithm::LeastConnections), "LEAST_REQUEST");
        assert_eq!(lb_policy_name(&LbAlgorithm::ConsistentHashing), "RING_HASH");
    }

    #[test]
    fn cluster_json_shape() {
        let up = Upstream::new("my-cluster".to_string());
        let cluster = build_cluster(&up, &[]);
        assert_eq!(cluster["name"].as_str(), Some("my-cluster"));
        assert_eq!(cluster["lb_policy"].as_str(), Some("ROUND_ROBIN"));
    }

    #[test]
    fn eds_empty_targets() {
        let assignment = build_cluster_load_assignment("test-cluster", &[]);
        assert_eq!(assignment["cluster_name"].as_str(), Some("test-cluster"));
        assert_eq!(assignment["endpoints"][0]["lb_endpoints"].as_array().map(|a| a.len()), Some(0));
    }
}

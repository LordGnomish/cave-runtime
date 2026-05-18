// SPDX-License-Identifier: AGPL-3.0-or-later
//! Kong Admin API — full CRUD for all gateway entities.
//!
//! Mounted at /admin/v1/ (mirrors Kong's :8001 admin port).
//! All responses use Kong-compatible JSON format.

use crate::models::*;
use crate::store::SharedStore;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

pub type AdminState = SharedStore;

// ── Admin router ──────────────────────────────────────────────────────────────

pub fn admin_router(store: SharedStore) -> Router {
    Router::new()
        // Root info
        .route("/", get(root_info))
        // Services
        .route("/services", get(list_services).post(create_service))
        .route("/services/{id_or_name}", get(get_service).patch(update_service).put(upsert_service).delete(delete_service))
        .route("/services/{id_or_name}/routes", get(list_routes_for_service))
        .route("/services/{id_or_name}/plugins", get(list_plugins_for_service))
        // Routes
        .route("/routes", get(list_routes).post(create_route))
        .route("/routes/{id_or_name}", get(get_route).patch(update_route).put(upsert_route).delete(delete_route))
        .route("/routes/{id_or_name}/plugins", get(list_plugins_for_route))
        // Upstreams
        .route("/upstreams", get(list_upstreams).post(create_upstream))
        .route("/upstreams/{id_or_name}", get(get_upstream).patch(update_upstream).put(upsert_upstream).delete(delete_upstream))
        .route("/upstreams/{id_or_name}/targets", get(list_targets).post(create_target))
        .route("/upstreams/{id_or_name}/targets/{target_id}", get(get_target).delete(delete_target))
        .route("/upstreams/{id_or_name}/targets/{target_id}/healthy", put(set_target_healthy))
        .route("/upstreams/{id_or_name}/targets/{target_id}/unhealthy", put(set_target_unhealthy))
        .route("/upstreams/{id_or_name}/health", get(get_upstream_health))
        // Consumers
        .route("/consumers", get(list_consumers).post(create_consumer))
        .route("/consumers/{id_or_name}", get(get_consumer).patch(update_consumer).delete(delete_consumer))
        .route("/consumers/{id_or_name}/plugins", get(list_plugins_for_consumer))
        // Consumer credentials
        .route("/consumers/{id_or_name}/key-auth", get(list_key_auth).post(create_key_auth))
        .route("/consumers/{id_or_name}/key-auth/{cred_id}", delete(delete_key_auth))
        .route("/consumers/{id_or_name}/jwt", get(list_jwt).post(create_jwt))
        .route("/consumers/{id_or_name}/jwt/{cred_id}", delete(delete_jwt))
        .route("/consumers/{id_or_name}/basic-auth", get(list_basic_auth).post(create_basic_auth))
        .route("/consumers/{id_or_name}/basic-auth/{cred_id}", delete(delete_basic_auth))
        .route("/consumers/{id_or_name}/hmac-auth", get(list_hmac_auth).post(create_hmac_auth))
        .route("/consumers/{id_or_name}/hmac-auth/{cred_id}", delete(delete_hmac_auth))
        .route("/consumers/{id_or_name}/acls", get(list_acls).post(create_acl))
        .route("/consumers/{id_or_name}/acls/{acl_id}", delete(delete_acl))
        // Plugins
        .route("/plugins", get(list_plugins).post(create_plugin))
        .route("/plugins/{id}", get(get_plugin).patch(update_plugin).delete(delete_plugin))
        .route("/plugins/enabled", get(list_enabled_plugins))
        .route("/plugins/schema/{plugin_name}", get(get_plugin_schema))
        // Certificates
        .route("/certificates", get(list_certificates).post(create_certificate))
        .route("/certificates/{id}", get(get_certificate).patch(update_certificate).delete(delete_certificate))
        .route("/certificates/{id}/snis", get(list_snis_for_cert))
        // SNIs
        .route("/snis", get(list_snis).post(create_sni))
        .route("/snis/{id_or_name}", get(get_sni).patch(update_sni).delete(delete_sni))
        // Tags
        .route("/tags", get(list_tags))
        .route("/tags/{tag}", get(list_entities_by_tag))
        // Node info
        .route("/status", get(node_status))
        .route("/schemas/{entity}", get(get_entity_schema))
        .with_state(store)
}

// ── Root ──────────────────────────────────────────────────────────────────────

async fn root_info() -> Json<Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "node_id": Uuid::new_v4(),
        "lua_version": "n/a",
        "tagline": "CAVE Gateway — Kong-compatible Admin API",
        "plugins": {
            "available_on_server": {
                "rate-limiting": true, "key-auth": true, "jwt": true, "oauth2": true,
                "basic-auth": true, "hmac-auth": true, "acl": true, "cors": true,
                "request-transformer": true, "response-transformer": true,
                "ip-restriction": true, "bot-detection": true, "request-size-limiting": true,
                "proxy-cache": true, "request-termination": true, "http-log": true,
                "file-log": true, "prometheus": true, "zipkin": true, "grpc-gateway": true,
            }
        }
    }))
}

async fn node_status(State(store): State<AdminState>) -> Json<Value> {
    Json(json!({
        "server": {
            "connections_accepted": 0u64,
            "connections_active": 0u64,
            "connections_handled": 0u64,
            "connections_reading": 0u64,
            "connections_waiting": 0u64,
            "connections_writing": 0u64,
            "total_requests": 0u64,
        },
        "database": {
            "reachable": true,
        },
        "memory": {
            "workers_lua_vms": [],
        },
        "configuration_hash": "unknown",
    }))
}

// ── Services ──────────────────────────────────────────────────────────────────

async fn list_services(State(store): State<AdminState>) -> Json<Value> {
    let services = store.list_services();
    Json(json!({"data": services, "next": null}))
}

async fn get_service(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_service_by_id_or_name(&id_or_name) {
        Some(svc) => Json(serde_json::to_value(svc).unwrap()).into_response(),
        None => not_found("service"),
    }
}

async fn create_service(
    State(store): State<AdminState>,
    Json(body): Json<CreateService>,
) -> impl IntoResponse {
    let (host, port, protocol) = if let Some(url) = &body.url {
        parse_url(url)
    } else {
        (body.host.clone(), body.port.unwrap_or(80), body.protocol.clone().unwrap_or(Protocol::Http))
    };

    let mut svc = Service::new(host, port, protocol);
    svc.name = body.name;
    if let Some(p) = body.path { svc.path = Some(p); }
    if let Some(r) = body.retries { svc.retries = r; }
    if let Some(t) = body.connect_timeout { svc.connect_timeout = t; }
    if let Some(t) = body.write_timeout { svc.write_timeout = t; }
    if let Some(t) = body.read_timeout { svc.read_timeout = t; }
    if let Some(e) = body.enabled { svc.enabled = e; }
    if let Some(tags) = body.tags { svc.tags = tags; }
    if let Some(v) = body.tls_verify { svc.tls_verify = Some(v); }

    store.insert_service(svc.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(svc).unwrap())).into_response()
}

async fn update_service(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateService>,
) -> impl IntoResponse {
    let mut svc = match store.get_service_by_id_or_name(&id_or_name) {
        Some(s) => s,
        None => return not_found("service"),
    };

    if let Some(n) = body.name { svc.name = Some(n); }
    if let Some(h) = body.protocol { svc.protocol = h; }
    if !body.host.is_empty() { svc.host = body.host; }
    if let Some(p) = body.port { svc.port = p; }
    if let Some(p) = body.path { svc.path = Some(p); }
    if let Some(r) = body.retries { svc.retries = r; }
    svc.updated_at = Utc::now();

    store.insert_service(svc.clone());
    Json(serde_json::to_value(svc).unwrap()).into_response()
}

async fn upsert_service(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateService>,
) -> Response {
    if store.get_service_by_id_or_name(&id_or_name).is_some() {
        update_service(State(store), Path(id_or_name), Json(body)).await.into_response()
    } else {
        create_service(State(store), Json(body)).await.into_response()
    }
}

async fn delete_service(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let id = store
        .get_service_by_id_or_name(&id_or_name)
        .map(|s| s.id);
    match id {
        Some(id) if store.delete_service(&id) => StatusCode::NO_CONTENT.into_response(),
        _ => not_found("service"),
    }
}

async fn list_routes_for_service(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_service_by_id_or_name(&id_or_name) {
        Some(svc) => {
            let routes = store.routes_for_service(&svc.id);
            Json(json!({"data": routes, "next": null})).into_response()
        }
        None => not_found("service"),
    }
}

async fn list_plugins_for_service(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_service_by_id_or_name(&id_or_name) {
        Some(svc) => {
            let plugins = store.plugins_for_service(&svc.id);
            Json(json!({"data": plugins, "next": null})).into_response()
        }
        None => not_found("service"),
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

async fn list_routes(State(store): State<AdminState>) -> Json<Value> {
    Json(json!({"data": store.list_routes(), "next": null}))
}

async fn get_route(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_route_by_id_or_name(&id_or_name) {
        Some(r) => Json(serde_json::to_value(r).unwrap()).into_response(),
        None => not_found("route"),
    }
}

async fn create_route(
    State(store): State<AdminState>,
    Json(body): Json<CreateRoute>,
) -> impl IntoResponse {
    let service_id = body.service.as_ref().and_then(|r| r.id);
    let mut route = Route::new(service_id.unwrap_or_default());
    if service_id.is_none() {
        route.service_id = None;
    }
    route.name = body.name;
    if let Some(p) = body.protocols { route.protocols = p; }
    if let Some(m) = body.methods { route.methods = Some(m); }
    if let Some(h) = body.hosts { route.hosts = Some(h); }
    if let Some(p) = body.paths { route.paths = Some(p); }
    if let Some(h) = body.headers { route.headers = Some(h); }
    if let Some(rp) = body.regex_priority { route.regex_priority = rp; }
    if let Some(sp) = body.strip_path { route.strip_path = sp; }
    if let Some(ph) = body.preserve_host { route.preserve_host = ph; }
    if let Some(ph) = body.path_handling { route.path_handling = ph; }
    if let Some(snis) = body.snis { route.snis = Some(snis); }
    if let Some(tags) = body.tags { route.tags = tags; }

    store.insert_route(route.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(route).unwrap())).into_response()
}

async fn update_route(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateRoute>,
) -> impl IntoResponse {
    let mut route = match store.get_route_by_id_or_name(&id_or_name) {
        Some(r) => r,
        None => return not_found("route"),
    };

    if let Some(n) = body.name { route.name = Some(n); }
    if let Some(p) = body.protocols { route.protocols = p; }
    if let Some(m) = body.methods { route.methods = Some(m); }
    if let Some(h) = body.hosts { route.hosts = Some(h); }
    if let Some(p) = body.paths { route.paths = Some(p); }
    if let Some(h) = body.headers { route.headers = Some(h); }
    if let Some(rp) = body.regex_priority { route.regex_priority = rp; }
    if let Some(sp) = body.strip_path { route.strip_path = sp; }
    route.updated_at = Utc::now();

    store.insert_route(route.clone());
    Json(serde_json::to_value(route).unwrap()).into_response()
}

async fn upsert_route(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateRoute>,
) -> Response {
    if store.get_route_by_id_or_name(&id_or_name).is_some() {
        update_route(State(store), Path(id_or_name), Json(body)).await.into_response()
    } else {
        create_route(State(store), Json(body)).await.into_response()
    }
}

async fn delete_route(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let id = store.get_route_by_id_or_name(&id_or_name).map(|r| r.id);
    match id {
        Some(id) if store.delete_route(&id) => StatusCode::NO_CONTENT.into_response(),
        _ => not_found("route"),
    }
}

async fn list_plugins_for_route(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_route_by_id_or_name(&id_or_name) {
        Some(r) => Json(json!({"data": store.plugins_for_route(&r.id), "next": null})).into_response(),
        None => not_found("route"),
    }
}

// ── Upstreams ─────────────────────────────────────────────────────────────────

async fn list_upstreams(State(store): State<AdminState>) -> Json<Value> {
    Json(json!({"data": store.list_upstreams(), "next": null}))
}

async fn get_upstream(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_upstream_by_id_or_name(&id_or_name) {
        Some(u) => Json(serde_json::to_value(u).unwrap()).into_response(),
        None => not_found("upstream"),
    }
}

async fn create_upstream(
    State(store): State<AdminState>,
    Json(body): Json<CreateUpstream>,
) -> impl IntoResponse {
    let mut up = Upstream::new(body.name);
    if let Some(a) = body.algorithm { up.algorithm = a; }
    if let Some(h) = body.hash_on { up.hash_on = h; }
    if let Some(s) = body.slots { up.slots = s; }
    if let Some(hc) = body.healthchecks { up.healthchecks = hc; }
    if let Some(tags) = body.tags { up.tags = tags; }

    store.insert_upstream(up.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(up).unwrap())).into_response()
}

async fn update_upstream(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateUpstream>,
) -> impl IntoResponse {
    let mut up = match store.get_upstream_by_id_or_name(&id_or_name) {
        Some(u) => u,
        None => return not_found("upstream"),
    };
    if let Some(a) = body.algorithm { up.algorithm = a; }
    if let Some(h) = body.hash_on { up.hash_on = h; }
    if let Some(hc) = body.healthchecks { up.healthchecks = hc; }
    up.updated_at = Utc::now();
    store.insert_upstream(up.clone());
    Json(serde_json::to_value(up).unwrap()).into_response()
}

async fn upsert_upstream(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateUpstream>,
) -> Response {
    if store.get_upstream_by_id_or_name(&id_or_name).is_some() {
        update_upstream(State(store), Path(id_or_name), Json(body)).await.into_response()
    } else {
        create_upstream(State(store), Json(body)).await.into_response()
    }
}

async fn delete_upstream(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let id = store.get_upstream_by_id_or_name(&id_or_name).map(|u| u.id);
    match id {
        Some(id) if store.delete_upstream(&id) => StatusCode::NO_CONTENT.into_response(),
        _ => not_found("upstream"),
    }
}

// ── Targets ───────────────────────────────────────────────────────────────────

async fn list_targets(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_upstream_by_id_or_name(&id_or_name) {
        Some(up) => {
            let targets = store.targets_for_upstream(&up.id);
            Json(json!({"data": targets, "next": null})).into_response()
        }
        None => not_found("upstream"),
    }
}

async fn get_target(
    State(store): State<AdminState>,
    Path((id_or_name, target_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    match store.targets.get(&target_id) {
        Some(t) => Json(serde_json::to_value(t.value().clone()).unwrap()).into_response(),
        None => not_found("target"),
    }
}

async fn create_target(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateTarget>,
) -> impl IntoResponse {
    let up = match store.get_upstream_by_id_or_name(&id_or_name) {
        Some(u) => u,
        None => return not_found("upstream"),
    };

    let mut t = Target::new(up.id, body.target, body.weight.unwrap_or(100));
    if let Some(tags) = body.tags { t.tags = tags; }
    store.insert_target(t.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(t).unwrap())).into_response()
}

async fn delete_target(
    State(store): State<AdminState>,
    Path((_, target_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if store.delete_target(&target_id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("target")
    }
}

async fn set_target_healthy(
    State(store): State<AdminState>,
    Path((id_or_name, target_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    StatusCode::NO_CONTENT.into_response() // health registry in proxy engine
}

async fn set_target_unhealthy(
    State(store): State<AdminState>,
    Path((id_or_name, target_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    StatusCode::NO_CONTENT.into_response()
}

async fn get_upstream_health(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_upstream_by_id_or_name(&id_or_name) {
        Some(up) => {
            let targets = store.targets_for_upstream(&up.id);
            let health_data: Vec<Value> = targets.iter().map(|t| {
                json!({
                    "id": t.id,
                    "target": t.target,
                    "weight": t.weight,
                    "health": "HEALTHY",
                    "data": {}
                })
            }).collect();
            Json(json!({"data": health_data, "node_id": Uuid::new_v4(), "now": Utc::now()})).into_response()
        }
        None => not_found("upstream"),
    }
}

// ── Consumers ─────────────────────────────────────────────────────────────────

async fn list_consumers(State(store): State<AdminState>) -> Json<Value> {
    Json(json!({"data": store.list_consumers(), "next": null}))
}

async fn get_consumer(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => Json(serde_json::to_value(c).unwrap()).into_response(),
        None => not_found("consumer"),
    }
}

async fn create_consumer(
    State(store): State<AdminState>,
    Json(body): Json<CreateConsumer>,
) -> impl IntoResponse {
    let mut c = Consumer::new(body.username, body.custom_id);
    if let Some(tags) = body.tags { c.tags = tags; }
    store.insert_consumer(c.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(c).unwrap())).into_response()
}

async fn update_consumer(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateConsumer>,
) -> impl IntoResponse {
    let mut c = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    if let Some(u) = body.username { c.username = Some(u); }
    if let Some(ci) = body.custom_id { c.custom_id = Some(ci); }
    if let Some(tags) = body.tags { c.tags = tags; }
    c.updated_at = Utc::now();
    store.insert_consumer(c.clone());
    Json(serde_json::to_value(c).unwrap()).into_response()
}

async fn delete_consumer(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let id = store.get_consumer_by_id_or_name(&id_or_name).map(|c| c.id);
    match id {
        Some(id) if store.delete_consumer(&id) => StatusCode::NO_CONTENT.into_response(),
        _ => not_found("consumer"),
    }
}

async fn list_plugins_for_consumer(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => {
            let plugins: Vec<Plugin> = store
                .list_plugins()
                .into_iter()
                .filter(|p| p.consumer_id == Some(c.id))
                .collect();
            Json(json!({"data": plugins, "next": null})).into_response()
        }
        None => not_found("consumer"),
    }
}

// ── Consumer credentials ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateKeyAuth {
    key: Option<String>,
    tags: Option<Vec<String>>,
    ttl: Option<u64>,
}

async fn list_key_auth(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    let creds: Vec<_> = store
        .key_auth
        .iter()
        .filter(|e| e.value().consumer_id == consumer.id)
        .map(|e| e.value().clone())
        .collect();
    Json(json!({"data": creds, "next": null})).into_response()
}

async fn create_key_auth(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateKeyAuth>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };

    let key = body.key.unwrap_or_else(|| {
        use rand::distributions::Alphanumeric;
        use rand::Rng;
        rand::thread_rng().sample_iter(&Alphanumeric).take(32).map(char::from).collect()
    });

    let cred = KeyAuthCredential {
        id: Uuid::new_v4(),
        consumer_id: consumer.id,
        key: key.clone(),
        tags: body.tags.unwrap_or_default(),
        ttl: body.ttl,
        created_at: Utc::now(),
    };

    store.key_auth.insert(cred.id, cred.clone());
    store.key_auth_idx.insert(key, cred.id);

    (StatusCode::CREATED, Json(serde_json::to_value(cred).unwrap())).into_response()
}

async fn delete_key_auth(
    State(store): State<AdminState>,
    Path((_, cred_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if let Some((_, cred)) = store.key_auth.remove(&cred_id) {
        store.key_auth_idx.remove(&cred.key);
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("key-auth credential")
    }
}

#[derive(Deserialize)]
struct CreateJwt {
    algorithm: Option<JwtAlgorithm>,
    key: Option<String>,
    rsa_public_key: Option<String>,
    secret: Option<String>,
    tags: Option<Vec<String>>,
}

async fn list_jwt(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    let creds: Vec<_> = store
        .jwt_creds
        .iter()
        .filter(|e| e.value().consumer_id == consumer.id)
        .map(|e| e.value().clone())
        .collect();
    Json(json!({"data": creds, "next": null})).into_response()
}

async fn create_jwt(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateJwt>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };

    let key = body.key.unwrap_or_else(|| Uuid::new_v4().to_string());
    let secret = body.secret.unwrap_or_else(|| {
        use rand::distributions::Alphanumeric;
        use rand::Rng;
        rand::thread_rng().sample_iter(&Alphanumeric).take(32).map(char::from).collect()
    });

    let cred = JwtCredential {
        id: Uuid::new_v4(),
        consumer_id: consumer.id,
        algorithm: body.algorithm.unwrap_or(JwtAlgorithm::HS256),
        key: key.clone(),
        rsa_public_key: body.rsa_public_key,
        secret: Some(secret),
        tags: body.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };

    store.jwt_creds.insert(cred.id, cred.clone());
    store.jwt_key_idx.insert(key, cred.id);

    (StatusCode::CREATED, Json(serde_json::to_value(cred).unwrap())).into_response()
}

async fn delete_jwt(
    State(store): State<AdminState>,
    Path((_, cred_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if let Some((_, cred)) = store.jwt_creds.remove(&cred_id) {
        store.jwt_key_idx.remove(&cred.key);
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("jwt credential")
    }
}

#[derive(Deserialize)]
struct CreateBasicAuth {
    username: String,
    password: String,
    tags: Option<Vec<String>>,
}

async fn list_basic_auth(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    let creds: Vec<_> = store
        .basic_auth
        .iter()
        .filter(|e| e.value().consumer_id == consumer.id)
        .map(|e| {
            let mut c = e.value().clone();
            c.password = "".to_string(); // never return password
            c
        })
        .collect();
    Json(json!({"data": creds, "next": null})).into_response()
}

async fn create_basic_auth(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateBasicAuth>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };

    let cred = BasicAuthCredential {
        id: Uuid::new_v4(),
        consumer_id: consumer.id,
        username: body.username.clone(),
        password: body.password, // store plain; real impl would bcrypt
        tags: body.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };

    store.basic_auth.insert(cred.id, cred.clone());
    store.basic_auth_idx.insert(body.username, cred.id);

    (StatusCode::CREATED, Json(serde_json::to_value(cred).unwrap())).into_response()
}

async fn delete_basic_auth(
    State(store): State<AdminState>,
    Path((_, cred_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if let Some((_, cred)) = store.basic_auth.remove(&cred_id) {
        store.basic_auth_idx.remove(&cred.username);
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("basic-auth credential")
    }
}

#[derive(Deserialize)]
struct CreateHmacAuth {
    username: String,
    secret: Option<String>,
    tags: Option<Vec<String>>,
}

async fn list_hmac_auth(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    let creds: Vec<_> = store
        .hmac_auth
        .iter()
        .filter(|e| e.value().consumer_id == consumer.id)
        .map(|e| e.value().clone())
        .collect();
    Json(json!({"data": creds, "next": null})).into_response()
}

async fn create_hmac_auth(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateHmacAuth>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    let secret = body.secret.unwrap_or_else(|| {
        use rand::distributions::Alphanumeric;
        use rand::Rng;
        rand::thread_rng().sample_iter(&Alphanumeric).take(32).map(char::from).collect()
    });
    let cred = HmacAuthCredential {
        id: Uuid::new_v4(),
        consumer_id: consumer.id,
        username: body.username.clone(),
        secret,
        tags: body.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };
    store.hmac_auth.insert(cred.id, cred.clone());
    store.hmac_auth_idx.insert(body.username, cred.id);
    (StatusCode::CREATED, Json(serde_json::to_value(cred).unwrap())).into_response()
}

async fn delete_hmac_auth(
    State(store): State<AdminState>,
    Path((_, cred_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if let Some((_, cred)) = store.hmac_auth.remove(&cred_id) {
        store.hmac_auth_idx.remove(&cred.username);
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("hmac-auth credential")
    }
}

#[derive(Deserialize)]
struct CreateAcl {
    group: String,
    tags: Option<Vec<String>>,
}

async fn list_acls(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    let acls: Vec<_> = store
        .acl_groups
        .iter()
        .filter(|e| e.value().consumer_id == consumer.id)
        .map(|e| e.value().clone())
        .collect();
    Json(json!({"data": acls, "next": null})).into_response()
}

async fn create_acl(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateAcl>,
) -> impl IntoResponse {
    let consumer = match store.get_consumer_by_id_or_name(&id_or_name) {
        Some(c) => c,
        None => return not_found("consumer"),
    };
    let acl = AclGroup {
        id: Uuid::new_v4(),
        consumer_id: consumer.id,
        group: body.group,
        tags: body.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };
    store.acl_groups.insert(acl.id, acl.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(acl).unwrap())).into_response()
}

async fn delete_acl(
    State(store): State<AdminState>,
    Path((_, acl_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    if store.acl_groups.remove(&acl_id).is_some() {
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("acl group")
    }
}

// ── Plugins ───────────────────────────────────────────────────────────────────

async fn list_plugins(State(store): State<AdminState>) -> Json<Value> {
    Json(json!({"data": store.list_plugins(), "next": null}))
}

async fn get_plugin(
    State(store): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match store.plugins.get(&id) {
        Some(p) => Json(serde_json::to_value(p.value().clone()).unwrap()).into_response(),
        None => not_found("plugin"),
    }
}

async fn create_plugin(
    State(store): State<AdminState>,
    Json(body): Json<CreatePlugin>,
) -> impl IntoResponse {
    let mut plugin = Plugin::new(body.name, body.config.unwrap_or(Value::Object(Default::default())));
    plugin.service_id = body.service.as_ref().and_then(|r| r.id);
    plugin.route_id = body.route.as_ref().and_then(|r| r.id);
    plugin.consumer_id = body.consumer.as_ref().and_then(|r| r.id);
    if let Some(e) = body.enabled { plugin.enabled = e; }
    if let Some(p) = body.protocols { plugin.protocols = p; }
    if let Some(tags) = body.tags { plugin.tags = tags; }

    store.insert_plugin(plugin.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(plugin).unwrap())).into_response()
}

async fn update_plugin(
    State(store): State<AdminState>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreatePlugin>,
) -> impl IntoResponse {
    let mut plugin = match store.plugins.get(&id) {
        Some(p) => p.value().clone(),
        None => return not_found("plugin"),
    };
    if let Some(c) = body.config { plugin.config = c; }
    if let Some(e) = body.enabled { plugin.enabled = e; }
    if let Some(p) = body.protocols { plugin.protocols = p; }
    if let Some(tags) = body.tags { plugin.tags = tags; }
    plugin.updated_at = Utc::now();
    store.insert_plugin(plugin.clone());
    Json(serde_json::to_value(plugin).unwrap()).into_response()
}

async fn delete_plugin(
    State(store): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if store.delete_plugin(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("plugin")
    }
}

async fn list_enabled_plugins() -> Json<Value> {
    Json(json!({
        "enabled_plugins": [
            "rate-limiting", "key-auth", "jwt", "oauth2", "basic-auth",
            "hmac-auth", "acl", "cors", "request-transformer", "response-transformer",
            "ip-restriction", "bot-detection", "request-size-limiting", "proxy-cache",
            "request-termination", "http-log", "file-log", "prometheus", "zipkin", "grpc-gateway"
        ]
    }))
}

async fn get_plugin_schema(Path(plugin_name): Path<String>) -> Json<Value> {
    Json(json!({
        "fields": [],
        "entity_checks": [],
    }))
}

// ── Certificates ──────────────────────────────────────────────────────────────

async fn list_certificates(State(store): State<AdminState>) -> Json<Value> {
    Json(json!({"data": store.list_certificates(), "next": null}))
}

async fn get_certificate(
    State(store): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match store.certificates.get(&id) {
        Some(c) => Json(serde_json::to_value(c.value().clone()).unwrap()).into_response(),
        None => not_found("certificate"),
    }
}

async fn create_certificate(
    State(store): State<AdminState>,
    Json(body): Json<CreateCertificate>,
) -> impl IntoResponse {
    let mut cert = Certificate::new(body.cert, body.key);
    cert.cert_alt = body.cert_alt;
    cert.key_alt = body.key_alt;
    if let Some(tags) = body.tags { cert.tags = tags; }

    // Auto-create SNIs if provided
    let cert_id = cert.id;
    store.insert_certificate(cert.clone());

    if let Some(sni_names) = body.snis {
        for name in sni_names {
            let sni = Sni::new(name, cert_id);
            store.insert_sni(sni);
        }
    }

    (StatusCode::CREATED, Json(serde_json::to_value(cert).unwrap())).into_response()
}

async fn update_certificate(
    State(store): State<AdminState>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateCertificate>,
) -> impl IntoResponse {
    let mut cert = match store.certificates.get(&id) {
        Some(c) => c.value().clone(),
        None => return not_found("certificate"),
    };
    cert.cert = body.cert;
    cert.key = body.key;
    cert.cert_alt = body.cert_alt;
    cert.key_alt = body.key_alt;
    cert.updated_at = Utc::now();
    store.insert_certificate(cert.clone());
    Json(serde_json::to_value(cert).unwrap()).into_response()
}

async fn delete_certificate(
    State(store): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if store.delete_certificate(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        not_found("certificate")
    }
}

async fn list_snis_for_cert(
    State(store): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let snis: Vec<_> = store
        .snis
        .iter()
        .filter(|e| e.value().certificate_id == id)
        .map(|e| e.value().clone())
        .collect();
    Json(json!({"data": snis, "next": null})).into_response()
}

// ── SNIs ──────────────────────────────────────────────────────────────────────

async fn list_snis(State(store): State<AdminState>) -> Json<Value> {
    Json(json!({"data": store.list_snis(), "next": null}))
}

async fn get_sni(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let sni = if let Ok(id) = id_or_name.parse::<Uuid>() {
        store.snis.get(&id).map(|e| e.value().clone())
    } else {
        store.get_sni_by_name(&id_or_name)
    };
    match sni {
        Some(s) => Json(serde_json::to_value(s).unwrap()).into_response(),
        None => not_found("sni"),
    }
}

async fn create_sni(
    State(store): State<AdminState>,
    Json(body): Json<CreateSni>,
) -> impl IntoResponse {
    let cert_id = match body.certificate.id {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, Json(json!({"message": "certificate.id required"}))).into_response(),
    };

    let mut sni = Sni::new(body.name, cert_id);
    if let Some(tags) = body.tags { sni.tags = tags; }
    store.insert_sni(sni.clone());
    (StatusCode::CREATED, Json(serde_json::to_value(sni).unwrap())).into_response()
}

async fn update_sni(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
    Json(body): Json<CreateSni>,
) -> impl IntoResponse {
    let sni = if let Ok(id) = id_or_name.parse::<Uuid>() {
        store.snis.get(&id).map(|e| e.value().clone())
    } else {
        store.get_sni_by_name(&id_or_name)
    };
    let mut sni = match sni {
        Some(s) => s,
        None => return not_found("sni"),
    };
    if let Some(cert_id) = body.certificate.id { sni.certificate_id = cert_id; }
    sni.updated_at = Utc::now();
    store.insert_sni(sni.clone());
    Json(serde_json::to_value(sni).unwrap()).into_response()
}

async fn delete_sni(
    State(store): State<AdminState>,
    Path(id_or_name): Path<String>,
) -> impl IntoResponse {
    let id = if let Ok(id) = id_or_name.parse::<Uuid>() {
        Some(id)
    } else {
        store.get_sni_by_name(&id_or_name).map(|s| s.id)
    };
    match id {
        Some(id) if store.delete_sni(&id) => StatusCode::NO_CONTENT.into_response(),
        _ => not_found("sni"),
    }
}

// ── Tags ──────────────────────────────────────────────────────────────────────

async fn list_tags(State(store): State<AdminState>) -> Json<Value> {
    let mut tags = std::collections::HashSet::new();
    for s in store.list_services() { for t in s.tags { tags.insert(t); } }
    for r in store.list_routes() { for t in r.tags { tags.insert(t); } }
    for u in store.list_upstreams() { for t in u.tags { tags.insert(t); } }
    for c in store.list_consumers() { for t in c.tags { tags.insert(t); } }
    let data: Vec<Value> = tags.into_iter().map(|t| json!({"tag": t, "entity_type": "mixed"})).collect();
    Json(json!({"data": data, "next": null}))
}

async fn list_entities_by_tag(
    State(store): State<AdminState>,
    Path(tag): Path<String>,
) -> Json<Value> {
    let mut data = Vec::new();
    for s in store.list_services() {
        if s.tags.contains(&tag) {
            data.push(json!({"entity_type": "services", "entity_id": s.id}));
        }
    }
    for r in store.list_routes() {
        if r.tags.contains(&tag) {
            data.push(json!({"entity_type": "routes", "entity_id": r.id}));
        }
    }
    Json(json!({"data": data, "next": null}))
}

async fn get_entity_schema(Path(entity): Path<String>) -> Json<Value> {
    Json(json!({"fields": [], "entity_checks": []}))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn not_found(entity: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"message": format!("{} not found", entity)})),
    )
        .into_response()
}

fn parse_url(url: &str) -> (String, u16, Protocol) {
    if let Ok(u) = url::Url::parse(url) {
        let protocol = match u.scheme() {
            "https" => Protocol::Https,
            "grpc" => Protocol::Grpc,
            "grpcs" => Protocol::Grpcs,
            "ws" => Protocol::Ws,
            "wss" => Protocol::Wss,
            _ => Protocol::Http,
        };
        let host = u.host_str().unwrap_or("localhost").to_string();
        let port = u.port().unwrap_or(match protocol {
            Protocol::Https | Protocol::Grpcs | Protocol::Wss => 443,
            _ => 80,
        });
        (host, port, protocol)
    } else {
        ("localhost".to_string(), 80, Protocol::Http)
    }
}

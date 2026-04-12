//! Admin API — full CRUD for services, routes, upstreams, targets, consumers, plugins.
//!
//! Route prefix: /admin
//!
//! Endpoints:
//!   /admin/services               GET (list), POST (create)
//!   /admin/services/:id           GET, PUT, DELETE
//!   /admin/routes                 GET, POST
//!   /admin/routes/:id             GET, PUT, DELETE
//!   /admin/upstreams              GET, POST
//!   /admin/upstreams/:id          GET, PUT, DELETE
//!   /admin/upstreams/:id/targets  GET, POST
//!   /admin/upstreams/:id/targets/:tid  GET, DELETE
//!   /admin/consumers              GET, POST
//!   /admin/consumers/:id          GET, PUT, DELETE
//!   /admin/consumers/:id/key-auth    GET, POST
//!   /admin/consumers/:id/jwt         GET, POST
//!   /admin/consumers/:id/basic-auth  GET, POST
//!   /admin/consumers/:id/hmac-auth   GET, POST
//!   /admin/plugins                GET, POST
//!   /admin/plugins/:id            GET, PUT, DELETE

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

type AppState = Arc<GatewayState>;

pub fn admin_router(state: AppState) -> Router {
    Router::new()
        // Services
        .route("/services", get(list_services).post(create_service))
        .route(
            "/services/:id",
            get(get_service).put(update_service).delete(delete_service),
        )
        // Routes
        .route("/routes", get(list_routes).post(create_route))
        .route(
            "/routes/:id",
            get(get_route).put(update_route).delete(delete_route),
        )
        // Upstreams
        .route("/upstreams", get(list_upstreams).post(create_upstream))
        .route(
            "/upstreams/:id",
            get(get_upstream).put(update_upstream).delete(delete_upstream),
        )
        // Targets
        .route(
            "/upstreams/:id/targets",
            get(list_targets).post(create_target),
        )
        .route(
            "/upstreams/:id/targets/:tid",
            get(get_target).delete(delete_target),
        )
        .route("/upstreams/:id/health", get(upstream_health))
        // Consumers
        .route("/consumers", get(list_consumers).post(create_consumer))
        .route(
            "/consumers/:id",
            get(get_consumer).put(update_consumer).delete(delete_consumer),
        )
        // Credentials
        .route(
            "/consumers/:id/key-auth",
            get(list_key_auth).post(create_key_auth),
        )
        .route(
            "/consumers/:id/jwt",
            get(list_jwt).post(create_jwt),
        )
        .route(
            "/consumers/:id/basic-auth",
            get(list_basic_auth).post(create_basic_auth),
        )
        .route(
            "/consumers/:id/hmac-auth",
            get(list_hmac_auth).post(create_hmac_auth),
        )
        // Plugins
        .route("/plugins", get(list_plugins).post(create_plugin))
        .route(
            "/plugins/:id",
            get(get_plugin).put(update_plugin).delete(delete_plugin),
        )
        .with_state(state)
}

// ─────────────────────────────────────────────
//  Services
// ─────────────────────────────────────────────

async fn list_services(State(s): State<AppState>) -> Json<ListResponse<Service>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(store.list_services().into_iter().cloned().collect()))
}

async fn create_service(
    State(s): State<AppState>,
    Json(req): Json<CreateServiceRequest>,
) -> (StatusCode, Json<Service>) {
    let now = Utc::now();
    let svc = Service {
        id: Uuid::new_v4(),
        name: req.name,
        protocol: req.protocol.unwrap_or_default(),
        host: req.host,
        port: req.port.unwrap_or(80),
        path: req.path,
        retries: req.retries.unwrap_or(5),
        connect_timeout: req.connect_timeout.unwrap_or(60_000),
        write_timeout: req.write_timeout.unwrap_or(60_000),
        read_timeout: req.read_timeout.unwrap_or(60_000),
        tags: req.tags.unwrap_or_default(),
        enabled: req.enabled.unwrap_or(true),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_service(svc.clone());
    (StatusCode::CREATED, Json(svc))
}

async fn get_service(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Service>, StatusCode> {
    s.store
        .read()
        .unwrap()
        .get_service(id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_service(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateServiceRequest>,
) -> Result<Json<Service>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let svc = store.services.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    svc.name = req.name;
    if let Some(proto) = req.protocol { svc.protocol = proto; }
    svc.host = req.host;
    if let Some(port) = req.port { svc.port = port; }
    svc.path = req.path;
    if let Some(r) = req.retries { svc.retries = r; }
    if let Some(t) = req.connect_timeout { svc.connect_timeout = t; }
    if let Some(t) = req.write_timeout { svc.write_timeout = t; }
    if let Some(t) = req.read_timeout { svc.read_timeout = t; }
    if let Some(tags) = req.tags { svc.tags = tags; }
    if let Some(en) = req.enabled { svc.enabled = en; }
    svc.updated_at = Utc::now();
    Ok(Json(svc.clone()))
}

async fn delete_service(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if s.store.write().unwrap().remove_service(id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─────────────────────────────────────────────
//  Routes
// ─────────────────────────────────────────────

async fn list_routes(State(s): State<AppState>) -> Json<ListResponse<Route>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(store.list_routes().into_iter().cloned().collect()))
}

async fn create_route(
    State(s): State<AppState>,
    Json(req): Json<CreateRouteRequest>,
) -> (StatusCode, Json<Route>) {
    let now = Utc::now();
    let route = Route {
        id: Uuid::new_v4(),
        name: req.name,
        service_id: req.service_id,
        protocols: req.protocols.unwrap_or_else(|| vec![Protocol::Http, Protocol::Https]),
        methods: req.methods.unwrap_or_default(),
        hosts: req.hosts.unwrap_or_default(),
        paths: req.paths.unwrap_or_default(),
        headers: req.headers.unwrap_or_default(),
        snis: req.snis.unwrap_or_default(),
        strip_path: req.strip_path.unwrap_or(true),
        preserve_host: req.preserve_host.unwrap_or(false),
        regex_priority: req.regex_priority.unwrap_or(0),
        path_handling: PathHandling::V0,
        tags: req.tags.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_route(route.clone());
    (StatusCode::CREATED, Json(route))
}

async fn get_route(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Route>, StatusCode> {
    s.store
        .read()
        .unwrap()
        .get_route(id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_route(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateRouteRequest>,
) -> Result<Json<Route>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let route = store.routes.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    route.name = req.name;
    route.service_id = req.service_id;
    if let Some(p) = req.protocols { route.protocols = p; }
    if let Some(m) = req.methods { route.methods = m; }
    if let Some(h) = req.hosts { route.hosts = h; }
    if let Some(p) = req.paths { route.paths = p; }
    if let Some(h) = req.headers { route.headers = h; }
    if let Some(s) = req.snis { route.snis = s; }
    if let Some(sp) = req.strip_path { route.strip_path = sp; }
    if let Some(ph) = req.preserve_host { route.preserve_host = ph; }
    if let Some(rp) = req.regex_priority { route.regex_priority = rp; }
    if let Some(t) = req.tags { route.tags = t; }
    route.updated_at = Utc::now();
    Ok(Json(route.clone()))
}

async fn delete_route(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if s.store.write().unwrap().remove_route(id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─────────────────────────────────────────────
//  Upstreams
// ─────────────────────────────────────────────

async fn list_upstreams(State(s): State<AppState>) -> Json<ListResponse<Upstream>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(store.list_upstreams().into_iter().cloned().collect()))
}

async fn create_upstream(
    State(s): State<AppState>,
    Json(req): Json<CreateUpstreamRequest>,
) -> (StatusCode, Json<Upstream>) {
    let now = Utc::now();
    let upstream = Upstream {
        id: Uuid::new_v4(),
        name: req.name,
        algorithm: req.algorithm.unwrap_or_default(),
        hash_on: req.hash_on.unwrap_or_default(),
        hash_fallback: HashFallback::None,
        hash_on_header: req.hash_on_header,
        healthchecks: HealthCheckConfig::default(),
        tags: req.tags.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_upstream(upstream.clone());
    (StatusCode::CREATED, Json(upstream))
}

async fn get_upstream(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Upstream>, StatusCode> {
    s.store
        .read()
        .unwrap()
        .get_upstream(id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_upstream(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateUpstreamRequest>,
) -> Result<Json<Upstream>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let upstream = store.upstreams.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    upstream.name = req.name;
    if let Some(a) = req.algorithm { upstream.algorithm = a; }
    if let Some(h) = req.hash_on { upstream.hash_on = h; }
    upstream.hash_on_header = req.hash_on_header;
    if let Some(t) = req.tags { upstream.tags = t; }
    upstream.updated_at = Utc::now();
    Ok(Json(upstream.clone()))
}

async fn delete_upstream(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if s.store.write().unwrap().remove_upstream(id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─────────────────────────────────────────────
//  Targets
// ─────────────────────────────────────────────

async fn list_targets(
    State(s): State<AppState>,
    Path(upstream_id): Path<Uuid>,
) -> Json<ListResponse<Target>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(
        store.targets_for_upstream(upstream_id).into_iter().cloned().collect(),
    ))
}

async fn create_target(
    State(s): State<AppState>,
    Path(upstream_id): Path<Uuid>,
    Json(req): Json<CreateTargetRequest>,
) -> (StatusCode, Json<Target>) {
    let now = Utc::now();
    let target = Target {
        id: Uuid::new_v4(),
        upstream_id,
        target: req.target,
        weight: req.weight.unwrap_or(100),
        health: TargetHealth::Healthy,
        tags: req.tags.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_target(target.clone());
    (StatusCode::CREATED, Json(target))
}

async fn get_target(
    State(s): State<AppState>,
    Path((upstream_id, tid)): Path<(Uuid, Uuid)>,
) -> Result<Json<Target>, StatusCode> {
    let store = s.store.read().unwrap();
    store
        .get_target(tid)
        .filter(|t| t.upstream_id == upstream_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_target(
    State(s): State<AppState>,
    Path((_upstream_id, tid)): Path<(Uuid, Uuid)>,
) -> StatusCode {
    if s.store.write().unwrap().remove_target(tid).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn upstream_health(
    State(s): State<AppState>,
    Path(upstream_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = s.store.read().unwrap();
    if store.get_upstream(upstream_id).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let targets = store.targets_for_upstream(upstream_id);
    let health: Vec<serde_json::Value> = targets
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "target": t.target,
                "health": t.health,
                "weight": t.weight,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "upstream_id": upstream_id,
        "data": health,
    })))
}

// ─────────────────────────────────────────────
//  Consumers
// ─────────────────────────────────────────────

async fn list_consumers(State(s): State<AppState>) -> Json<ListResponse<Consumer>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(store.list_consumers().into_iter().cloned().collect()))
}

async fn create_consumer(
    State(s): State<AppState>,
    Json(req): Json<CreateConsumerRequest>,
) -> (StatusCode, Json<Consumer>) {
    let now = Utc::now();
    let consumer = Consumer {
        id: Uuid::new_v4(),
        username: req.username,
        custom_id: req.custom_id,
        tags: req.tags.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_consumer(consumer.clone());
    (StatusCode::CREATED, Json(consumer))
}

async fn get_consumer(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Consumer>, StatusCode> {
    s.store
        .read()
        .unwrap()
        .get_consumer(id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_consumer(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateConsumerRequest>,
) -> Result<Json<Consumer>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let consumer = store.consumers.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    consumer.username = req.username;
    consumer.custom_id = req.custom_id;
    if let Some(t) = req.tags { consumer.tags = t; }
    consumer.updated_at = Utc::now();
    Ok(Json(consumer.clone()))
}

async fn delete_consumer(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if s.store.write().unwrap().remove_consumer(id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ─────────────────────────────────────────────
//  Credentials: key-auth
// ─────────────────────────────────────────────

async fn list_key_auth(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
) -> Json<ListResponse<KeyAuthCredential>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(
        store.key_auth_for_consumer(consumer_id).into_iter().cloned().collect(),
    ))
}

async fn create_key_auth(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
    Json(req): Json<CreateKeyAuthRequest>,
) -> (StatusCode, Json<KeyAuthCredential>) {
    let cred = KeyAuthCredential {
        id: Uuid::new_v4(),
        consumer_id,
        key: req.key.unwrap_or_else(|| format!("cave_{}", Uuid::new_v4().simple())),
        tags: req.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };
    s.store.write().unwrap().key_auth_creds.insert(cred.id, cred.clone());
    (StatusCode::CREATED, Json(cred))
}

// ─────────────────────────────────────────────
//  Credentials: jwt
// ─────────────────────────────────────────────

async fn list_jwt(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
) -> Json<ListResponse<JwtCredential>> {
    let store = s.store.read().unwrap();
    let creds: Vec<JwtCredential> = store
        .jwt_creds
        .values()
        .filter(|c| c.consumer_id == consumer_id)
        .cloned()
        .collect();
    Json(ListResponse::new(creds))
}

async fn create_jwt(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
    Json(req): Json<CreateJwtRequest>,
) -> (StatusCode, Json<JwtCredential>) {
    let cred = JwtCredential {
        id: Uuid::new_v4(),
        consumer_id,
        key: req.key.unwrap_or_else(|| Uuid::new_v4().to_string()),
        secret: req.secret.unwrap_or_else(|| Uuid::new_v4().to_string()),
        algorithm: req.algorithm.unwrap_or_else(|| "HS256".to_string()),
        tags: req.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };
    s.store.write().unwrap().jwt_creds.insert(cred.id, cred.clone());
    (StatusCode::CREATED, Json(cred))
}

// ─────────────────────────────────────────────
//  Credentials: basic-auth
// ─────────────────────────────────────────────

async fn list_basic_auth(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
) -> Json<ListResponse<BasicAuthCredential>> {
    let store = s.store.read().unwrap();
    let creds: Vec<BasicAuthCredential> = store
        .basic_auth_creds
        .values()
        .filter(|c| c.consumer_id == consumer_id)
        .cloned()
        .collect();
    Json(ListResponse::new(creds))
}

async fn create_basic_auth(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
    Json(req): Json<CreateBasicAuthRequest>,
) -> (StatusCode, Json<BasicAuthCredential>) {
    let cred = BasicAuthCredential {
        id: Uuid::new_v4(),
        consumer_id,
        username: req.username,
        password_hash: req.password, // in prod: bcrypt/argon2 hash
        tags: req.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };
    s.store.write().unwrap().basic_auth_creds.insert(cred.id, cred.clone());
    (StatusCode::CREATED, Json(cred))
}

// ─────────────────────────────────────────────
//  Credentials: hmac-auth
// ─────────────────────────────────────────────

async fn list_hmac_auth(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
) -> Json<ListResponse<HmacAuthCredential>> {
    let store = s.store.read().unwrap();
    let creds: Vec<HmacAuthCredential> = store
        .hmac_auth_creds
        .values()
        .filter(|c| c.consumer_id == consumer_id)
        .cloned()
        .collect();
    Json(ListResponse::new(creds))
}

async fn create_hmac_auth(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
    Json(req): Json<CreateHmacAuthRequest>,
) -> (StatusCode, Json<HmacAuthCredential>) {
    let cred = HmacAuthCredential {
        id: Uuid::new_v4(),
        consumer_id,
        username: req.username,
        secret: req.secret.unwrap_or_else(|| Uuid::new_v4().to_string()),
        tags: req.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };
    s.store.write().unwrap().hmac_auth_creds.insert(cred.id, cred.clone());
    (StatusCode::CREATED, Json(cred))
}

// ─────────────────────────────────────────────
//  Plugins
// ─────────────────────────────────────────────

async fn list_plugins(State(s): State<AppState>) -> Json<ListResponse<Plugin>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(store.list_plugins().into_iter().cloned().collect()))
}

async fn create_plugin(
    State(s): State<AppState>,
    Json(req): Json<CreatePluginRequest>,
) -> (StatusCode, Json<Plugin>) {
    let now = Utc::now();
    let plugin = Plugin {
        id: Uuid::new_v4(),
        name: req.name,
        service_id: req.service_id,
        route_id: req.route_id,
        consumer_id: req.consumer_id,
        config: req.config.unwrap_or(serde_json::json!({})),
        enabled: req.enabled.unwrap_or(true),
        protocols: vec![Protocol::Http, Protocol::Https],
        tags: req.tags.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_plugin(plugin.clone());
    (StatusCode::CREATED, Json(plugin))
}

async fn get_plugin(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Plugin>, StatusCode> {
    s.store
        .read()
        .unwrap()
        .get_plugin(id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_plugin(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreatePluginRequest>,
) -> Result<Json<Plugin>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let plugin = store.plugins.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;
    plugin.name = req.name;
    plugin.service_id = req.service_id;
    plugin.route_id = req.route_id;
    plugin.consumer_id = req.consumer_id;
    if let Some(c) = req.config { plugin.config = c; }
    if let Some(e) = req.enabled { plugin.enabled = e; }
    if let Some(t) = req.tags { plugin.tags = t; }
    plugin.updated_at = Utc::now();
    Ok(Json(plugin.clone()))
}

async fn delete_plugin(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if s.store.write().unwrap().remove_plugin(id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

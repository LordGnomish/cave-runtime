//! HTTP routes for cave-knative (Serving + Eventing).

use crate::models::{
    CreateBrokerRequest, CreateChannelRequest, CreateServiceRequest, CreateSourceRequest,
    CreateSubscriptionRequest, CreateTriggerRequest, ScaleRequest, SendEventRequest,
    UpdateServiceRequest,
};
use crate::KnativeState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

type AppState = Arc<KnativeState>;

pub fn create_router(state: Arc<KnativeState>) -> Router {
    Router::new()
        .route("/api/knative/health", get(health))
        // ── Serving: Services ────────────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/services",
            get(list_services).post(create_service),
        )
        .route(
            "/api/knative/namespaces/{ns}/services/{name}",
            get(get_service).put(update_service).delete(delete_service),
        )
        // ── Serving: Revisions ───────────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/revisions",
            get(list_revisions),
        )
        .route(
            "/api/knative/namespaces/{ns}/revisions/{rev}",
            get(get_revision).delete(delete_revision),
        )
        .route(
            "/api/knative/namespaces/{ns}/revisions/{rev}/scale",
            post(scale_revision),
        )
        // ── Serving: Routes ──────────────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/routes",
            get(list_routes),
        )
        .route(
            "/api/knative/namespaces/{ns}/routes/{name}",
            get(get_route),
        )
        // ── Eventing: Brokers ────────────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/brokers",
            get(list_brokers).post(create_broker),
        )
        .route(
            "/api/knative/namespaces/{ns}/brokers/{name}",
            get(get_broker).delete(delete_broker),
        )
        .route(
            "/api/knative/namespaces/{ns}/brokers/{name}/events",
            get(get_broker_events).post(send_event),
        )
        // ── Eventing: Triggers ───────────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/triggers",
            get(list_triggers).post(create_trigger),
        )
        .route(
            "/api/knative/namespaces/{ns}/triggers/{name}",
            get(get_trigger).delete(delete_trigger),
        )
        // ── Eventing: Sources ────────────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/sources",
            get(list_sources).post(create_source),
        )
        .route(
            "/api/knative/namespaces/{ns}/sources/{name}",
            get(get_source).delete(delete_source),
        )
        // ── Eventing: Channels ───────────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/channels",
            get(list_channels).post(create_channel),
        )
        .route(
            "/api/knative/namespaces/{ns}/channels/{name}",
            get(get_channel).delete(delete_channel),
        )
        // ── Eventing: Subscriptions ──────────────────────────────────────────
        .route(
            "/api/knative/namespaces/{ns}/subscriptions",
            get(list_subscriptions).post(create_subscription),
        )
        .route(
            "/api/knative/namespaces/{ns}/subscriptions/{name}",
            get(get_subscription).delete(delete_subscription),
        )
        // ── Stats ─────────────────────────────────────────────────────────────
        .route("/api/knative/stats", get(stats))
        .with_state(state)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn err_resp(err: &crate::error::KnativeError) -> (StatusCode, Json<serde_json::Value>) {
    let code = StatusCode::from_u16(err.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(json!({ "error": err.to_string() })))
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-knative",
        "status": "ok",
        "components": ["serving", "eventing"]
    }))
}

// ── Services ──────────────────────────────────────────────────────────────────

async fn list_services(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    let svcs = s.serving.list_services(&ns);
    Json(json!({ "count": svcs.len(), "services": svcs }))
}

async fn create_service(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    Json(mut req): Json<CreateServiceRequest>,
) -> impl IntoResponse {
    req.namespace = ns;
    match s.serving.create_service(req) {
        Ok(svc) => (StatusCode::CREATED, Json(json!(svc))).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn get_service(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.serving.get_service(&ns, &name) {
        Ok(svc) => Json(json!(svc)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn update_service(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
    Json(req): Json<UpdateServiceRequest>,
) -> impl IntoResponse {
    match s.serving.update_service(&ns, &name, req) {
        Ok(svc) => Json(json!(svc)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn delete_service(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.serving.delete_service(&ns, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

// ── Revisions ─────────────────────────────────────────────────────────────────

async fn list_revisions(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    // List all revisions in this namespace (no service filter in this endpoint)
    let all: Vec<_> = s
        .serving
        .list_services(&ns)
        .iter()
        .flat_map(|svc| {
            s.serving
                .list_revisions_for_service(&ns, &svc.name)
        })
        .collect();
    Json(json!({ "count": all.len(), "revisions": all }))
}

async fn get_revision(
    State(s): State<AppState>,
    Path((ns, rev)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.serving.get_revision(&ns, &rev) {
        Ok(r) => Json(json!(r)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn delete_revision(
    State(s): State<AppState>,
    Path((ns, rev)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.serving.delete_revision(&ns, &rev) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn scale_revision(
    State(s): State<AppState>,
    Path((ns, rev)): Path<(String, String)>,
    Json(req): Json<ScaleRequest>,
) -> impl IntoResponse {
    match s.serving.scale_revision(&ns, &rev, req.replicas) {
        Ok(r) => Json(json!(r)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

async fn list_routes(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    let routes = s.serving.list_routes(&ns);
    Json(json!({ "count": routes.len(), "routes": routes }))
}

async fn get_route(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.serving.get_route(&ns, &name) {
        Ok(r) => Json(json!(r)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

// ── Brokers ───────────────────────────────────────────────────────────────────

async fn list_brokers(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    let brokers = s.eventing.list_brokers(&ns);
    Json(json!({ "count": brokers.len(), "brokers": brokers }))
}

async fn create_broker(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    Json(mut req): Json<CreateBrokerRequest>,
) -> impl IntoResponse {
    req.namespace = ns;
    match s.eventing.create_broker(req) {
        Ok(b) => (StatusCode::CREATED, Json(json!(b))).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn get_broker(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.get_broker(&ns, &name) {
        Ok(b) => Json(json!(b)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn delete_broker(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.delete_broker(&ns, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn send_event(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
    Json(req): Json<SendEventRequest>,
) -> impl IntoResponse {
    match s.eventing.send_event(&ns, &name, req) {
        Ok(event) => (StatusCode::ACCEPTED, Json(json!(event))).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn get_broker_events(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> Json<serde_json::Value> {
    let events = s.eventing.get_broker_events(&ns, &name, 100);
    Json(json!({ "count": events.len(), "events": events }))
}

// ── Triggers ──────────────────────────────────────────────────────────────────

async fn list_triggers(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    let triggers = s.eventing.list_triggers(&ns);
    Json(json!({ "count": triggers.len(), "triggers": triggers }))
}

async fn create_trigger(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    Json(mut req): Json<CreateTriggerRequest>,
) -> impl IntoResponse {
    req.namespace = ns;
    match s.eventing.create_trigger(req) {
        Ok(t) => (StatusCode::CREATED, Json(json!(t))).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn get_trigger(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.get_trigger(&ns, &name) {
        Ok(t) => Json(json!(t)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn delete_trigger(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.delete_trigger(&ns, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

// ── Sources ───────────────────────────────────────────────────────────────────

async fn list_sources(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    let sources = s.eventing.list_sources(&ns);
    Json(json!({ "count": sources.len(), "sources": sources }))
}

async fn create_source(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    Json(mut req): Json<CreateSourceRequest>,
) -> impl IntoResponse {
    req.namespace = ns;
    match s.eventing.create_source(req) {
        Ok(src) => (StatusCode::CREATED, Json(json!(src))).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn get_source(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.get_source(&ns, &name) {
        Ok(src) => Json(json!(src)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn delete_source(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.delete_source(&ns, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

// ── Channels ──────────────────────────────────────────────────────────────────

async fn list_channels(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    let channels = s.eventing.list_channels(&ns);
    Json(json!({ "count": channels.len(), "channels": channels }))
}

async fn create_channel(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    Json(mut req): Json<CreateChannelRequest>,
) -> impl IntoResponse {
    req.namespace = ns;
    match s.eventing.create_channel(req) {
        Ok(ch) => (StatusCode::CREATED, Json(json!(ch))).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn get_channel(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.get_channel(&ns, &name) {
        Ok(ch) => Json(json!(ch)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn delete_channel(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.delete_channel(&ns, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

// ── Subscriptions ─────────────────────────────────────────────────────────────

async fn list_subscriptions(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Json<serde_json::Value> {
    let subs = s.eventing.list_subscriptions(&ns);
    Json(json!({ "count": subs.len(), "subscriptions": subs }))
}

async fn create_subscription(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    Json(mut req): Json<CreateSubscriptionRequest>,
) -> impl IntoResponse {
    req.namespace = ns;
    match s.eventing.create_subscription(req) {
        Ok(sub) => (StatusCode::CREATED, Json(json!(sub))).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn get_subscription(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.get_subscription(&ns, &name) {
        Ok(sub) => Json(json!(sub)).into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

async fn delete_subscription(
    State(s): State<AppState>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.eventing.delete_subscription(&ns, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(&e).into_response(),
    }
}

// ── Stats ─────────────────────────────────────────────────────────────────────

async fn stats(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "total_events_processed": s.eventing.total_events(),
    }))
}

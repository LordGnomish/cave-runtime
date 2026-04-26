//! Developer portal API (Gravitee-inspired) + API lifecycle + monetization.
//!
//! Routes:
//!   /portal/apis                          GET — API catalog
//!   /portal/apis/{id}                      GET — API details
//!   /portal/apis/{id}/documentation        GET, POST
//!   /portal/subscriptions                 GET, POST
//!   /portal/subscriptions/{id}             GET, DELETE
//!   /portal/consumers/{id}/subscriptions   GET
//!   /portal/consumers/{id}/usage           GET
//!   /portal/usage                         GET — global usage summary
//!
//!   /admin/services/{id}/versions          GET, POST
//!   /admin/services/{id}/versions/{vid}     PUT
//!   /admin/services/{id}/versions/{vid}/deprecate  POST
//!   /admin/services/{id}/versions/{vid}/retire     POST

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

type AppState = Arc<GatewayState>;

/// Developer portal routes (consumer-facing)
pub fn portal_router(state: AppState) -> Router {
    Router::new()
        // API catalog
        .route("/apis", get(list_apis))
        .route("/apis/{id}", get(get_api))
        .route("/apis/{id}/documentation", get(list_docs).post(create_doc))
        // Subscriptions
        .route("/subscriptions", get(list_subscriptions).post(create_subscription))
        .route("/subscriptions/{id}", get(get_subscription).delete(cancel_subscription))
        .route("/consumers/{id}/subscriptions", get(consumer_subscriptions))
        // Usage / monetization
        .route("/consumers/{id}/usage", get(consumer_usage))
        .route("/usage", get(global_usage))
        .with_state(state)
}

/// API lifecycle routes (admin-facing, nested under /admin/services)
pub fn lifecycle_router(state: AppState) -> Router {
    Router::new()
        .route("/services/{id}/versions", get(list_versions).post(create_version))
        .route("/services/{id}/versions/{vid}", put(update_version))
        .route("/services/{id}/versions/{vid}/deprecate", post(deprecate_version))
        .route("/services/{id}/versions/{vid}/retire", post(retire_version))
        .with_state(state)
}

// ─────────────────────────────────────────────
//  API catalog
// ─────────────────────────────────────────────

async fn list_apis(State(s): State<AppState>) -> Json<ListResponse<Service>> {
    let store = s.store.read().unwrap();
    // The "API catalog" exposes enabled services with their version info
    let services: Vec<Service> = store
        .list_services()
        .into_iter()
        .filter(|svc| svc.enabled)
        .cloned()
        .collect();
    Json(ListResponse::new(services))
}

async fn get_api(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = s.store.read().unwrap();
    let svc = store.get_service(id).ok_or(StatusCode::NOT_FOUND)?;
    let versions = store.versions_for_service(id);
    let docs = store.docs_for_service(id);
    let subs = store.subscriptions_for_service(id);

    Ok(Json(serde_json::json!({
        "service": svc,
        "versions": versions,
        "documentation_count": docs.len(),
        "subscriber_count": subs.len(),
    })))
}

// ─────────────────────────────────────────────
//  Documentation
// ─────────────────────────────────────────────

async fn list_docs(
    State(s): State<AppState>,
    Path(service_id): Path<Uuid>,
) -> Json<ListResponse<ApiDoc>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(
        store.docs_for_service(service_id).into_iter().cloned().collect(),
    ))
}

async fn create_doc(
    State(s): State<AppState>,
    Path(service_id): Path<Uuid>,
    Json(req): Json<CreateDocRequest>,
) -> (StatusCode, Json<ApiDoc>) {
    let now = Utc::now();
    let doc = ApiDoc {
        id: Uuid::new_v4(),
        service_id,
        title: req.title,
        content: req.content,
        format: req.format.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_doc(doc.clone());
    (StatusCode::CREATED, Json(doc))
}

// ─────────────────────────────────────────────
//  Subscriptions
// ─────────────────────────────────────────────

async fn list_subscriptions(State(s): State<AppState>) -> Json<ListResponse<PortalSubscription>> {
    let store = s.store.read().unwrap();
    let subs: Vec<PortalSubscription> = store.subscriptions.values().cloned().collect();
    Json(ListResponse::new(subs))
}

async fn create_subscription(
    State(s): State<AppState>,
    Json(req): Json<CreateSubscriptionRequest>,
) -> (StatusCode, Json<PortalSubscription>) {
    let now = Utc::now();

    // Auto-provision an API key for the subscription
    let api_key = Some(format!("cave_{}", Uuid::new_v4().simple()));

    // Also create a key-auth credential linked to the consumer
    if let Some(ref key) = api_key {
        let cred = KeyAuthCredential {
            id: Uuid::new_v4(),
            consumer_id: req.consumer_id,
            key: key.clone(),
            tags: vec!["portal-subscription".to_string()],
            created_at: now,
        };
        s.store.write().unwrap().key_auth_creds.insert(cred.id, cred);
    }

    let sub = PortalSubscription {
        id: Uuid::new_v4(),
        consumer_id: req.consumer_id,
        service_id: req.service_id,
        plan: req.plan.unwrap_or_default(),
        status: SubscriptionStatus::Active,
        api_key,
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_subscription(sub.clone());
    (StatusCode::CREATED, Json(sub))
}

async fn get_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PortalSubscription>, StatusCode> {
    s.store
        .read()
        .unwrap()
        .subscriptions
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn cancel_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = s.store.write().unwrap();
    if let Some(sub) = store.subscriptions.get_mut(&id) {
        sub.status = SubscriptionStatus::Cancelled;
        sub.updated_at = Utc::now();
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn consumer_subscriptions(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
) -> Json<ListResponse<PortalSubscription>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(
        store
            .subscriptions_for_consumer(consumer_id)
            .into_iter()
            .cloned()
            .collect(),
    ))
}

// ─────────────────────────────────────────────
//  Usage / Monetization
// ─────────────────────────────────────────────

async fn consumer_usage(
    State(s): State<AppState>,
    Path(consumer_id): Path<Uuid>,
) -> Json<Vec<UsageRecord>> {
    let store = s.store.read().unwrap();
    Json(store.usage_for_consumer(consumer_id).into_iter().cloned().collect())
}

async fn global_usage(State(s): State<AppState>) -> Json<serde_json::Value> {
    let store = s.store.read().unwrap();
    let total: u64 = store.usage_records.iter().map(|r| r.request_count).sum();
    let total_bytes: u64 = store.usage_records.iter().map(|r| r.response_bytes).sum();

    Json(serde_json::json!({
        "total_requests": total,
        "total_bytes": total_bytes,
        "record_count": store.usage_records.len(),
    }))
}

// ─────────────────────────────────────────────
//  API Lifecycle (versioning)
// ─────────────────────────────────────────────

async fn list_versions(
    State(s): State<AppState>,
    Path(service_id): Path<Uuid>,
) -> Json<ListResponse<ApiVersion>> {
    let store = s.store.read().unwrap();
    Json(ListResponse::new(
        store.versions_for_service(service_id).into_iter().cloned().collect(),
    ))
}

async fn create_version(
    State(s): State<AppState>,
    Path(service_id): Path<Uuid>,
    Json(req): Json<CreateVersionRequest>,
) -> (StatusCode, Json<ApiVersion>) {
    let now = Utc::now();
    let version = ApiVersion {
        id: Uuid::new_v4(),
        service_id,
        version: req.version,
        status: ApiVersionStatus::Active,
        deprecated_at: None,
        sunset_at: None,
        changelog: req.changelog.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    s.store.write().unwrap().add_version(version.clone());
    (StatusCode::CREATED, Json(version))
}

async fn update_version(
    State(s): State<AppState>,
    Path((_service_id, vid)): Path<(Uuid, Uuid)>,
    Json(req): Json<CreateVersionRequest>,
) -> Result<Json<ApiVersion>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let version = store.api_versions.get_mut(&vid).ok_or(StatusCode::NOT_FOUND)?;
    version.version = req.version;
    if let Some(c) = req.changelog { version.changelog = c; }
    version.updated_at = Utc::now();
    Ok(Json(version.clone()))
}

async fn deprecate_version(
    State(s): State<AppState>,
    Path((_service_id, vid)): Path<(Uuid, Uuid)>,
) -> Result<Json<ApiVersion>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let version = store.api_versions.get_mut(&vid).ok_or(StatusCode::NOT_FOUND)?;
    version.status = ApiVersionStatus::Deprecated;
    version.deprecated_at = Some(Utc::now());
    version.updated_at = Utc::now();
    Ok(Json(version.clone()))
}

async fn retire_version(
    State(s): State<AppState>,
    Path((_service_id, vid)): Path<(Uuid, Uuid)>,
) -> Result<Json<ApiVersion>, StatusCode> {
    let mut store = s.store.write().unwrap();
    let version = store.api_versions.get_mut(&vid).ok_or(StatusCode::NOT_FOUND)?;
    if version.status != ApiVersionStatus::Deprecated {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    version.status = ApiVersionStatus::Retired;
    version.updated_at = Utc::now();
    Ok(Json(version.clone()))
}

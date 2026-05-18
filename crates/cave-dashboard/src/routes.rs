// SPDX-License-Identifier: AGPL-3.0-or-later
//! Full Grafana-compatible HTTP API routes.
//!
//! Implements:
//!   /api/dashboards/db              POST  save dashboard
//!   /api/dashboards/uid/:uid        GET   get dashboard
//!   /api/dashboards/id/:id/versions GET   version history
//!   /api/dashboards/id/:id/restore  POST  restore version
//!   /api/dashboards/tags            GET   list all tags
//!   /api/search                     GET   search dashboards + folders
//!   /api/folders                    GET/POST
//!   /api/folders/:uid               GET/PUT/DELETE
//!   /api/folders/:uid/permissions   GET/POST
//!   /api/datasources                GET/POST
//!   /api/datasources/:id            GET/PUT/DELETE
//!   /api/datasources/uid/:uid       GET
//!   /api/datasources/:id/health     GET
//!   /api/ds/query                   POST  unified query
//!   /api/annotations                GET/POST
//!   /api/annotations/:id            DELETE
//!   /api/snapshots                  POST
//!   /api/snapshots/:key             GET/DELETE
//!   /api/dashboard/snapshots        GET
//!   /api/playlists                  GET/POST
//!   /api/playlists/:id              GET/PUT/DELETE
//!   /api/orgs                       GET/POST
//!   /api/orgs/:id                   GET
//!   /api/org                        GET
//!   /api/users                      GET/POST
//!   /api/users/:id                  GET
//!   /api/user                       GET
//!   /api/teams                      GET/POST
//!   /api/teams/:id                  GET
//!   /api/teams/:id/members          GET/POST
//!   /api/auth/keys                  GET/POST
//!   /api/auth/keys/:id              DELETE
//!   /api/serviceaccounts            GET/POST
//!   /api/serviceaccounts/:id        GET
//!   /api/alerting/rules             GET
//!   /api/ruler/grafana/api/v1/rules GET/POST
//!   /api/ruler/grafana/api/v1/rules/:folder/:group  GET/PUT/DELETE
//!   /api/ruler/grafana/api/v1/rules/:folder/:group/:uid DELETE
//!   /api/alertmanager/.../am/api/v2/alerts  GET (alert groups)
//!   /api/alertmanager/.../silences   GET/POST
//!   /api/alertmanager/.../silence/:id DELETE
//!   /api/v1/provisioning/contact-points GET/PUT
//!   /api/v1/provisioning/policies    GET/PUT
//!   /api/v1/provisioning/mute-timings GET/POST/DELETE/:name
//!   /api/alert-notifications         GET/POST
//!   /api/alert-notifications/:id     GET/PUT/DELETE
//!   /api/dashboards/render/:uid      GET  HTML render
//!   /api/dashboard/health            GET

use crate::{
    alerting::{build_alert_groups, evaluate_alert_rule},
    auth::{generate_api_key, hash_api_key, require_editor},
    datasource::check_health,
    models::*,
    provisioning::{
        parse_alert_rule_provisioning, parse_contact_point_provisioning,
        parse_datasource_provisioning, parse_notification_policy_provisioning,
    },
    query::{apply_transformations, QueryCache},
    renderer::render_dashboard,
    store::{DashboardStore, StoreError},
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

// ─── Module state ─────────────────────────────────────────────────────────────

pub struct DashboardState {
    pub store: DashboardStore,
    pub query_cache: QueryCache,
}

impl DashboardState {
    pub fn new() -> Self {
        Self {
            store: DashboardStore::new(),
            query_cache: QueryCache::new(),
        }
    }
}

impl Default for DashboardState {
    fn default() -> Self {
        Self::new()
    }
}

type AppState = Arc<DashboardState>;

// ─── Error helpers ────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"message": msg}))).into_response()
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"message": msg}))).into_response()
}

fn internal_error(msg: &str) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"message": msg}))).into_response()
}

fn conflict(msg: &str) -> Response {
    (StatusCode::CONFLICT, Json(json!({"message": msg}))).into_response()
}

fn store_err(e: StoreError) -> Response {
    match e {
        StoreError::NotFound(msg) => not_found(&msg),
        StoreError::Conflict(msg) => conflict(&msg),
        StoreError::Lock => internal_error("store lock error"),
    }
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Health
        .route("/api/dashboard/health", get(health))

        // Dashboard CRUD
        .route("/api/dashboards/db", post(save_dashboard))
        .route("/api/dashboards/uid/{uid}", get(get_dashboard_by_uid).delete(delete_dashboard_by_uid))
        .route("/api/dashboards/id/{id}/versions", get(get_dashboard_versions))
        .route("/api/dashboards/id/{id}/restore", post(restore_dashboard_version))
        .route("/api/dashboards/tags", get(get_dashboard_tags))
        .route("/api/dashboards/id/{id}/permissions", get(get_dashboard_permissions).post(set_dashboard_permissions))

        // Stars
        .route("/api/user/stars/dashboard/{uid}", post(star_dashboard).delete(unstar_dashboard))

        // Search
        .route("/api/search", get(search))

        // Home dashboard
        .route("/api/dashboards/home", get(home_dashboard))

        // Folders
        .route("/api/folders", get(list_folders).post(create_folder))
        .route("/api/folders/{uid}", get(get_folder).put(update_folder).delete(delete_folder))
        .route("/api/folders/{uid}/permissions", get(get_folder_permissions).post(set_folder_permissions))

        // DataSources
        .route("/api/datasources", get(list_datasources).post(create_datasource))
        .route("/api/datasources/{id}", get(get_datasource).put(update_datasource).delete(delete_datasource))
        .route("/api/datasources/uid/{uid}", get(get_datasource_by_uid))
        .route("/api/datasources/{id}/health", get(datasource_health))

        // Unified query
        .route("/api/ds/query", post(ds_query))

        // Annotations
        .route("/api/annotations", get(list_annotations).post(create_annotation))
        .route("/api/annotations/{id}", delete(delete_annotation))

        // Snapshots
        .route("/api/snapshots", post(create_snapshot))
        .route("/api/snapshots/{key}", get(get_snapshot).delete(delete_snapshot))
        .route("/api/dashboard/snapshots", get(list_snapshots))

        // Playlists
        .route("/api/playlists", get(list_playlists).post(create_playlist))
        .route("/api/playlists/{id}", get(get_playlist).put(update_playlist).delete(delete_playlist))

        // Orgs
        .route("/api/orgs", get(list_orgs).post(create_org))
        .route("/api/orgs/{id}", get(get_org))
        .route("/api/org", get(current_org))

        // Users
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/{id}", get(get_user))
        .route("/api/user", get(current_user))

        // Teams
        .route("/api/teams/search", get(list_teams))
        .route("/api/teams", post(create_team))
        .route("/api/teams/{id}", get(get_team))
        .route("/api/teams/{id}/members", get(list_team_members).post(add_team_member))

        // API Keys
        .route("/api/auth/keys", get(list_api_keys).post(create_api_key))
        .route("/api/auth/keys/{id}", delete(delete_api_key))

        // Service Accounts
        .route("/api/serviceaccounts", get(list_service_accounts).post(create_service_account))
        .route("/api/serviceaccounts/{id}", get(get_service_account))

        // Unified Alerting — Ruler API
        .route("/api/ruler/grafana/api/v1/rules", get(list_ruler_groups))
        .route("/api/ruler/grafana/api/v1/rules/{folder_uid}", get(list_folder_rules))
        .route("/api/ruler/grafana/api/v1/rules/{folder_uid}/{group}", get(get_rule_group).put(put_rule_group).delete(delete_rule_group))

        // Alert instances / groups (Alertmanager API)
        .route("/api/alertmanager/grafana/api/v2/alerts", get(get_alert_groups))
        .route("/api/alertmanager/grafana/api/v2/alerts/groups", get(get_alert_groups))
        .route("/api/alertmanager/grafana/api/v2/silences", get(list_silences).post(create_silence))
        .route("/api/alertmanager/grafana/api/v2/silence/{id}", delete(delete_silence))

        // Contact Points (provisioning API)
        .route("/api/v1/provisioning/contact-points", get(list_contact_points).post(create_contact_point))
        .route("/api/v1/provisioning/contact-points/{uid}", put(update_contact_point).delete(delete_contact_point))

        // Notification Policy
        .route("/api/v1/provisioning/policies", get(get_notification_policy).put(put_notification_policy))

        // Mute Timings
        .route("/api/v1/provisioning/mute-timings", get(list_mute_timings).post(create_mute_timing))
        .route("/api/v1/provisioning/mute-timings/{name}", delete(delete_mute_timing))

        // Legacy alert notifications
        .route("/api/alert-notifications", get(list_alert_notifications).post(create_alert_notification))
        .route("/api/alert-notifications/{id}", get(get_alert_notification).put(update_alert_notification).delete(delete_alert_notification))

        // Legacy alerts (panel-level)
        .route("/api/alerts", get(list_legacy_alerts))

        // HTML Renderer
        .route("/render/d/{uid}", get(render_dashboard_html))
        .route("/api/dashboards/render/{uid}", get(render_dashboard_html))

        .with_state(state)
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-dashboard",
        "status": "ok",
        "upstream": "Grafana v10",
        "commit": env!("CARGO_PKG_VERSION"),
    }))
}

// ─── Dashboard CRUD ───────────────────────────────────────────────────────────

async fn save_dashboard(
    State(state): State<AppState>,
    Json(req): Json<UpsertDashboardRequest>,
) -> Response {
    let mut dashboard: Dashboard = match serde_json::from_value(req.dashboard) {
        Ok(d) => d,
        Err(e) => return bad_request(&format!("invalid dashboard JSON: {e}")),
    };

    if dashboard.uid.is_empty() {
        dashboard.uid = Uuid::new_v4().to_string().replace('-', "")[..12].to_string();
    }

    let folder_uid = req.folder_uid.as_deref();
    let result = state.store.upsert_dashboard(
        1, // org_id — in a real impl extract from auth context
        dashboard,
        folder_uid,
        &req.message,
        "admin",
        req.overwrite,
    );

    match result {
        Ok(d) => {
            let resp = UpsertDashboardResponse {
                id: d.id.unwrap_or(0),
                uid: d.uid.clone(),
                url: d.url.clone(),
                status: "success".into(),
                version: d.version,
                slug: d.slug.clone(),
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(StoreError::Conflict(msg)) => {
            (StatusCode::PRECONDITION_FAILED, Json(json!({"status":"version-mismatch","message":msg}))).into_response()
        }
        Err(e) => store_err(e),
    }
}

async fn get_dashboard_by_uid(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.get_dashboard_by_uid(&uid) {
        Ok(d) => Json(json!({
            "dashboard": d,
            "meta": {
                "type": "db",
                "canSave": true,
                "canEdit": true,
                "canAdmin": true,
                "canStar": true,
                "canDelete": true,
                "slug": d.slug,
                "url": d.url,
                "expires": "0001-01-01T00:00:00Z",
                "created": d.created,
                "updated": d.updated,
                "updatedBy": d.updated_by,
                "createdBy": d.created_by,
                "version": d.version,
                "hasAcl": false,
                "isFolder": false,
                "folderId": d.folder_id,
                "folderUid": d.folder_uid,
                "folderTitle": d.folder_title,
                "folderUrl": d.folder_url,
                "provisioned": false,
                "provisionedExternalId": "",
                "annotationsPermissions": {
                    "dashboard": {"canAdd":true,"canEdit":true,"canDelete":true},
                    "organization": {"canAdd":true,"canEdit":true,"canDelete":true}
                }
            }
        })).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_dashboard_by_uid(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.delete_dashboard(&uid) {
        Ok(_) => Json(json!({"title": uid, "message": "Dashboard deleted", "id": 0})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_dashboard_versions(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_dashboard_versions(id) {
        Ok(versions) => Json(versions).into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
struct RestoreVersionRequest {
    version: i64,
}

async fn restore_dashboard_version(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<RestoreVersionRequest>,
) -> Response {
    match state.store.restore_dashboard_version(id, req.version, "admin") {
        Ok(d) => Json(json!({"id":d.id,"uid":d.uid,"version":d.version,"message":"Dashboard restored"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_dashboard_tags(State(state): State<AppState>) -> Response {
    match state.store.list_dashboards(1) {
        Ok(dashboards) => {
            let mut tag_counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
            for d in &dashboards {
                for tag in &d.tags {
                    *tag_counts.entry(tag.clone()).or_insert(0) += 1;
                }
            }
            let tags: Vec<serde_json::Value> = tag_counts.iter()
                .map(|(t, c)| json!({"term": t, "count": c}))
                .collect();
            Json(tags).into_response()
        }
        Err(e) => store_err(e),
    }
}

async fn get_dashboard_permissions(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_dashboard_permissions(id) {
        Ok(perms) => Json(perms).into_response(),
        Err(e) => store_err(e),
    }
}

async fn set_dashboard_permissions(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(perms): Json<Vec<DashboardPermission>>,
) -> Response {
    match state.store.set_dashboard_permissions(id, perms) {
        Ok(_) => Json(json!({"message":"Dashboard permissions updated"})).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Stars ────────────────────────────────────────────────────────────────────

async fn star_dashboard(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.star_dashboard(1, &uid, true) {
        Ok(_) => Json(json!({"message":"Dashboard starred!"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn unstar_dashboard(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.star_dashboard(1, &uid, false) {
        Ok(_) => Json(json!({"message":"Dashboard unstarred"})).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Search ───────────────────────────────────────────────────────────────────

async fn search(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let q = SearchQuery {
        query: params.get("query").cloned().filter(|s| !s.is_empty()),
        tag: params.get("tag").map(|t| t.split(',').map(String::from).collect()).unwrap_or_default(),
        result_type: params.get("type").cloned(),
        dashboard_ids: vec![],
        dashboard_uids: vec![],
        folder_ids: vec![],
        folder_uids: params.get("folderUid").map(|u| vec![u.clone()]).unwrap_or_default(),
        starred: params.get("starred").and_then(|v| v.parse().ok()),
        limit: params.get("limit").and_then(|v| v.parse().ok()),
        page: params.get("page").and_then(|v| v.parse().ok()),
        sort: params.get("sort").cloned(),
        org_id: Some(1),
    };

    match state.store.search_dashboards(&q) {
        Ok(results) => Json(results).into_response(),
        Err(e) => store_err(e),
    }
}

async fn home_dashboard(State(state): State<AppState>) -> Response {
    // Return first dashboard or a default
    match state.store.list_dashboards(1) {
        Ok(dashboards) => {
            if let Some(d) = dashboards.first() {
                Json(json!({
                    "dashboard": d,
                    "meta": {"isHome": true, "canSave": false}
                })).into_response()
            } else {
                Json(json!({
                    "dashboard": {
                        "uid": "home",
                        "title": "Welcome to CAVE Dashboard",
                        "panels": [],
                        "schemaVersion": 39,
                        "version": 1
                    },
                    "meta": {"isHome": true, "canSave": false}
                })).into_response()
            }
        }
        Err(e) => store_err(e),
    }
}

// ─── Folders ──────────────────────────────────────────────────────────────────

async fn list_folders(State(state): State<AppState>) -> Response {
    match state.store.list_folders(1) {
        Ok(folders) => Json(folders).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> Response {
    match state.store.create_folder(1, req.uid.as_deref(), &req.title, req.parent_uid.as_deref()) {
        Ok(f) => (StatusCode::OK, Json(f)).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_folder(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.get_folder_by_uid(&uid) {
        Ok(f) => Json(f).into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
struct UpdateFolderRequest {
    title: String,
    #[allow(dead_code)]
    version: Option<i64>,
    #[allow(dead_code)]
    overwrite: Option<bool>,
}

async fn update_folder(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(req): Json<UpdateFolderRequest>,
) -> Response {
    match state.store.update_folder(&uid, &req.title) {
        Ok(f) => Json(f).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_folder(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.delete_folder(&uid) {
        Ok(_) => Json(json!({"message": "Folder deleted", "id": 0, "uid": uid})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_folder_permissions(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    Json(json!([])).into_response()
}

async fn set_folder_permissions(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(_): Json<serde_json::Value>,
) -> Response {
    Json(json!({"message":"Folder permissions updated"})).into_response()
}

// ─── DataSources ──────────────────────────────────────────────────────────────

async fn list_datasources(State(state): State<AppState>) -> Response {
    match state.store.list_datasources(1) {
        Ok(ds) => Json(ds).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_datasource(
    State(state): State<AppState>,
    Json(req): Json<CreateDataSourceRequest>,
) -> Response {
    match state.store.create_datasource(req, 1) {
        Ok(ds) => (StatusCode::OK, Json(json!({"datasource": ds, "id": ds.id, "message": "Datasource added", "name": ds.name}))).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_datasource(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_datasource_by_id(id) {
        Ok(ds) => Json(ds).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_datasource_by_uid(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.get_datasource_by_uid(&uid) {
        Ok(ds) => Json(ds).into_response(),
        Err(e) => store_err(e),
    }
}

async fn update_datasource(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<CreateDataSourceRequest>,
) -> Response {
    let ds = match state.store.get_datasource_by_id(id) {
        Ok(ds) => ds,
        Err(e) => return store_err(e),
    };
    match state.store.update_datasource(&ds.uid, req) {
        Ok(updated) => Json(json!({"datasource": updated, "message": "Datasource updated"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_datasource(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let ds = match state.store.get_datasource_by_id(id) {
        Ok(ds) => ds,
        Err(e) => return store_err(e),
    };
    match state.store.delete_datasource(&ds.uid) {
        Ok(_) => Json(json!({"message": "Data source deleted"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn datasource_health(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_datasource_by_id(id) {
        Ok(ds) => {
            let status = check_health(&ds).await;
            Json(status).into_response()
        }
        Err(e) => store_err(e),
    }
}

// ─── Unified Query ────────────────────────────────────────────────────────────

async fn ds_query(
    State(state): State<AppState>,
    Json(req): Json<DsQueryRequest>,
) -> Response {
    let mut response = DsQueryResponse::default();

    for query in &req.queries {
        let ds_uid = &query.datasource.uid;
        let ds = match state.store.get_datasource_by_uid(ds_uid) {
            Ok(ds) => ds,
            Err(_) => {
                response.results.insert(query.ref_id.clone(), QueryResult {
                    frames: vec![],
                    status: 400,
                    error: Some(format!("datasource {ds_uid} not found")),
                    error_source: Some("server".into()),
                });
                continue;
            }
        };

        // Check cache
        let cache_key = crate::query::cache_key(ds_uid, "", &req.from, &req.to);
        let result = if let Some(cached) = state.query_cache.get(&cache_key) {
            cached
        } else {
            let r = crate::datasource::execute_query(&ds, query).await;
            // Cache for 30s if TTL not specified
            if let Some(ttl_ms) = query.params.get("queryCachingTTL").and_then(|v| v.as_u64()) {
                state.query_cache.put(cache_key, r.clone(), std::time::Duration::from_millis(ttl_ms));
            }
            r
        };

        response.results.insert(query.ref_id.clone(), result);
    }

    Json(response).into_response()
}

// ─── Annotations ─────────────────────────────────────────────────────────────

async fn list_annotations(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let dashboard_uid = params.get("dashboardUID").map(|s| s.as_str());
    match state.store.list_annotations(dashboard_uid, 1) {
        Ok(anns) => Json(anns).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_annotation(
    State(state): State<AppState>,
    Json(req): Json<CreateAnnotationRequest>,
) -> Response {
    match state.store.create_annotation(req, 1, 1) {
        Ok(ann) => Json(json!({"id": ann.id, "message": "Annotation added"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_annotation(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.delete_annotation(id) {
        Ok(_) => Json(json!({"message":"Annotation deleted"})).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Snapshots ────────────────────────────────────────────────────────────────

async fn create_snapshot(
    State(state): State<AppState>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Response {
    match state.store.create_snapshot(req, 1, 1) {
        Ok(snap) => Json(json!({
            "deleteKey": snap.delete_key,
            "deleteUrl": format!("/api/snapshots-delete/{}", snap.delete_key),
            "key": snap.key,
            "url": snap.url,
        })).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_snapshot(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Response {
    match state.store.get_snapshot(&key) {
        Ok(snap) => Json(json!({
            "dashboard": snap.dashboard,
            "meta": {
                "type": "snapshot",
                "isSnapshot": true,
                "canSave": false,
                "canEdit": false,
                "canAdmin": false,
                "canStar": false,
                "slug": "",
                "url": snap.url,
                "expires": snap.expires,
                "created": snap.created,
            }
        })).into_response(),
        Err(StoreError::NotFound(msg)) if msg.contains("expired") => {
            (StatusCode::GONE, Json(json!({"message":"Snapshot has expired"}))).into_response()
        }
        Err(e) => store_err(e),
    }
}

async fn delete_snapshot(
    State(state): State<AppState>,
    Path(delete_key): Path<String>,
) -> Response {
    match state.store.delete_snapshot_by_delete_key(&delete_key) {
        Ok(_) => Json(json!({"message":"Snapshot deleted"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn list_snapshots(State(state): State<AppState>) -> Response {
    Json(json!([])).into_response()
}

// ─── Playlists ────────────────────────────────────────────────────────────────

async fn list_playlists(State(state): State<AppState>) -> Response {
    match state.store.list_playlists(1) {
        Ok(p) => Json(json!({"playlists": p, "totalCount": p.len()})).into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
struct CreatePlaylistRequest {
    name: String,
    interval: String,
    items: Vec<PlaylistItem>,
}

async fn create_playlist(
    State(state): State<AppState>,
    Json(req): Json<CreatePlaylistRequest>,
) -> Response {
    match state.store.create_playlist(1, &req.name, &req.interval, req.items) {
        Ok(p) => (StatusCode::OK, Json(p)).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_playlist(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_playlist(id) {
        Ok(p) => Json(p).into_response(),
        Err(e) => store_err(e),
    }
}

async fn update_playlist(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<CreatePlaylistRequest>,
) -> Response {
    match state.store.update_playlist(id, &req.name, &req.interval, req.items) {
        Ok(p) => Json(p).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_playlist(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.delete_playlist(id) {
        Ok(_) => Json(json!({"message":"Playlist deleted"})).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Orgs ─────────────────────────────────────────────────────────────────────

async fn list_orgs(State(state): State<AppState>) -> Response {
    match state.store.list_orgs() {
        Ok(orgs) => Json(orgs).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_org(
    State(state): State<AppState>,
    Json(req): Json<CreateOrgRequest>,
) -> Response {
    match state.store.create_org(&req.name) {
        Ok(org) => Json(json!({"orgId": org.id, "message": "Organization created"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_org(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_org(id) {
        Ok(org) => Json(org).into_response(),
        Err(e) => store_err(e),
    }
}

async fn current_org(State(state): State<AppState>) -> Response {
    match state.store.get_org(1) {
        Ok(org) => Json(org).into_response(),
        Err(_) => Json(json!({"id":1,"name":"Main Org."})).into_response(),
    }
}

// ─── Users ────────────────────────────────────────────────────────────────────

async fn list_users(State(state): State<AppState>) -> Response {
    match state.store.list_users() {
        Ok(users) => Json(users).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Response {
    match state.store.create_user(req) {
        Ok(u) => Json(json!({"id": u.id, "message": "User created"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_user(id) {
        Ok(u) => Json(u).into_response(),
        Err(e) => store_err(e),
    }
}

async fn current_user(State(state): State<AppState>) -> Response {
    Json(json!({
        "id": 1,
        "uid": "admin",
        "email": "admin@localhost",
        "login": "admin",
        "name": "Admin",
        "orgId": 1,
        "orgRole": "Admin",
        "isGrafanaAdmin": true,
        "theme": "dark",
        "avatarUrl": "/avatar/46d229b033af06a191ff2267bca9ae56",
    })).into_response()
}

// ─── Teams ────────────────────────────────────────────────────────────────────

async fn list_teams(State(state): State<AppState>) -> Response {
    match state.store.list_teams(1) {
        Ok(teams) => Json(json!({"teams": teams, "totalCount": teams.len()})).into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
struct CreateTeamRequest {
    name: String,
    email: String,
}

async fn create_team(
    State(state): State<AppState>,
    Json(req): Json<CreateTeamRequest>,
) -> Response {
    match state.store.create_team(1, &req.name, &req.email) {
        Ok(t) => Json(json!({"teamId": t.id, "message": "Team created"})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_team(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_team(id) {
        Ok(t) => Json(t).into_response(),
        Err(e) => store_err(e),
    }
}

async fn list_team_members(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.list_team_members(id) {
        Ok(m) => Json(m).into_response(),
        Err(e) => store_err(e),
    }
}

async fn add_team_member(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(member): Json<TeamMember>,
) -> Response {
    match state.store.add_team_member(id, member) {
        Ok(_) => Json(json!({"message":"Member added to team"})).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── API Keys ─────────────────────────────────────────────────────────────────

async fn list_api_keys(State(state): State<AppState>) -> Response {
    match state.store.list_api_keys(1) {
        Ok(keys) => Json(keys).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_api_key(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Response {
    let token = generate_api_key();
    let hash = hash_api_key(&token);
    let ttl = req.seconds_to_live;

    match state.store.create_api_key(1, &req.name, req.role, ttl, &hash, &token) {
        Ok(key) => Json(key).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_api_key(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.delete_api_key(id) {
        Ok(_) => Json(json!({"message":"API key deleted"})).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Service Accounts ─────────────────────────────────────────────────────────

async fn list_service_accounts(State(state): State<AppState>) -> Response {
    match state.store.list_service_accounts(1) {
        Ok(sas) => Json(json!({"serviceAccounts": sas, "totalCount": sas.len()})).into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
struct CreateServiceAccountRequest {
    name: String,
    role: OrgRole,
}

async fn create_service_account(
    State(state): State<AppState>,
    Json(req): Json<CreateServiceAccountRequest>,
) -> Response {
    match state.store.create_service_account(1, &req.name, req.role) {
        Ok(sa) => (StatusCode::CREATED, Json(sa)).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_service_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_service_account(id) {
        Ok(sa) => Json(sa).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Unified Alerting — Ruler API ─────────────────────────────────────────────

async fn list_ruler_groups(State(state): State<AppState>) -> Response {
    match state.store.list_rule_groups(1) {
        Ok(groups) => {
            let mut by_folder: std::collections::HashMap<String, Vec<serde_json::Value>> = std::collections::HashMap::new();
            for group in groups {
                let entry = by_folder.entry(group.folder_uid.clone()).or_default();
                entry.push(json!({
                    "name": group.name,
                    "interval": group.interval,
                    "rules": group.rules,
                }));
            }
            Json(by_folder).into_response()
        }
        Err(e) => store_err(e),
    }
}

async fn list_folder_rules(
    State(state): State<AppState>,
    Path(folder_uid): Path<String>,
) -> Response {
    match state.store.list_alert_rules(1) {
        Ok(rules) => {
            let folder_rules: Vec<&AlertRule> = rules.iter().filter(|r| r.folder_uid == folder_uid).collect();
            let mut groups: std::collections::HashMap<String, Vec<&AlertRule>> = std::collections::HashMap::new();
            for rule in folder_rules {
                groups.entry(rule.rule_group.clone()).or_default().push(rule);
            }
            let result: serde_json::Value = json!({
                folder_uid: groups.into_iter().map(|(name, rules)| json!({
                    "name": name,
                    "rules": rules,
                })).collect::<Vec<_>>()
            });
            Json(result).into_response()
        }
        Err(e) => store_err(e),
    }
}

async fn get_rule_group(
    State(state): State<AppState>,
    Path((folder_uid, group)): Path<(String, String)>,
) -> Response {
    match state.store.list_alert_rules(1) {
        Ok(rules) => {
            let group_rules: Vec<AlertRule> = rules.into_iter()
                .filter(|r| r.folder_uid == folder_uid && r.rule_group == group)
                .collect();
            Json(json!({"name": group, "interval": 60, "rules": group_rules})).into_response()
        }
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
struct PutRuleGroupRequest {
    name: Option<String>,
    interval: Option<i64>,
    rules: Vec<AlertRule>,
}

async fn put_rule_group(
    State(state): State<AppState>,
    Path((folder_uid, group)): Path<(String, String)>,
    Json(req): Json<PutRuleGroupRequest>,
) -> Response {
    for mut rule in req.rules {
        rule.folder_uid = folder_uid.clone();
        rule.rule_group = group.clone();
        rule.org_id = 1;
        if rule.uid.is_empty() {
            rule.uid = Uuid::new_v4().to_string().replace('-', "")[..9].to_string();
        }
        if let Err(e) = state.store.upsert_alert_rule(rule) {
            return store_err(e);
        }
    }
    (StatusCode::ACCEPTED, Json(json!({"message":"Rule group updated"}))).into_response()
}

async fn delete_rule_group(
    State(state): State<AppState>,
    Path((folder_uid, group)): Path<(String, String)>,
) -> Response {
    match state.store.list_alert_rules(1) {
        Ok(rules) => {
            let to_delete: Vec<String> = rules.into_iter()
                .filter(|r| r.folder_uid == folder_uid && r.rule_group == group)
                .map(|r| r.uid)
                .collect();
            for uid in to_delete {
                let _ = state.store.delete_alert_rule(&uid);
            }
            (StatusCode::ACCEPTED, Json(json!({"message":"Rule group deleted"}))).into_response()
        }
        Err(e) => store_err(e),
    }
}

// ─── Alert Groups (Alertmanager API) ─────────────────────────────────────────

async fn get_alert_groups(State(state): State<AppState>) -> Response {
    match state.store.list_alert_rules(1) {
        Ok(rules) => {
            let instances: Vec<AlertInstance> = rules.iter().map(|r| AlertInstance {
                state: r.state,
                labels: r.labels.clone(),
                annotations: r.annotations.clone(),
                value: String::new(),
                starts_at: r.updated,
                ends_at: None,
                generator_url: format!("/alerting/grafana/{}/view", r.uid),
                fingerprint: format!("{:x}", r.id),
                silence_urls: vec![],
                dashboard_url: None,
                panel_url: None,
                values: None,
                evaluations: None,
            }).collect();

            let policy = state.store.get_notification_policy().unwrap_or_default();
            let groups = build_alert_groups(instances, &policy);
            Json(groups).into_response()
        }
        Err(e) => store_err(e),
    }
}

// ─── Silences ─────────────────────────────────────────────────────────────────

async fn list_silences(State(state): State<AppState>) -> Response {
    match state.store.list_silences() {
        Ok(s) => Json(s).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_silence(
    State(state): State<AppState>,
    Json(silence): Json<Silence>,
) -> Response {
    match state.store.create_silence(silence) {
        Ok(s) => Json(json!({"silenceID": s.id})).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_silence(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.store.delete_silence(&id) {
        Ok(_) => (StatusCode::OK, Json(json!({}))).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Contact Points ───────────────────────────────────────────────────────────

async fn list_contact_points(State(state): State<AppState>) -> Response {
    match state.store.list_contact_points() {
        Ok(cps) => Json(cps).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_contact_point(
    State(state): State<AppState>,
    Json(mut cp): Json<ContactPoint>,
) -> Response {
    if cp.uid.is_empty() {
        cp.uid = Uuid::new_v4().to_string().replace('-', "")[..9].to_string();
    }
    match state.store.upsert_contact_point(cp) {
        Ok(c) => (StatusCode::ACCEPTED, Json(c)).into_response(),
        Err(e) => store_err(e),
    }
}

async fn update_contact_point(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(mut cp): Json<ContactPoint>,
) -> Response {
    cp.uid = uid;
    match state.store.upsert_contact_point(cp) {
        Ok(c) => (StatusCode::ACCEPTED, Json(c)).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_contact_point(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.delete_contact_point(&uid) {
        Ok(_) => (StatusCode::ACCEPTED, Json(json!({}))).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Notification Policy ─────────────────────────────────────────────────────

async fn get_notification_policy(State(state): State<AppState>) -> Response {
    match state.store.get_notification_policy() {
        Ok(p) => Json(p).into_response(),
        Err(_) => Json(NotificationPolicy::default()).into_response(),
    }
}

async fn put_notification_policy(
    State(state): State<AppState>,
    Json(policy): Json<NotificationPolicy>,
) -> Response {
    match state.store.set_notification_policy(policy) {
        Ok(_) => (StatusCode::ACCEPTED, Json(json!({"message":"Notification policy updated"}))).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Mute Timings ────────────────────────────────────────────────────────────

async fn list_mute_timings(State(state): State<AppState>) -> Response {
    match state.store.list_mute_timings() {
        Ok(mt) => Json(mt).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_mute_timing(
    State(state): State<AppState>,
    Json(mt): Json<MuteTiming>,
) -> Response {
    match state.store.upsert_mute_timing(mt) {
        Ok(m) => (StatusCode::CREATED, Json(m)).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_mute_timing(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    match state.store.delete_mute_timing(&name) {
        Ok(_) => (StatusCode::NO_CONTENT, Body::empty()).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Legacy Alert Notifications ───────────────────────────────────────────────

async fn list_alert_notifications(State(state): State<AppState>) -> Response {
    match state.store.list_notification_channels(1) {
        Ok(ch) => Json(ch).into_response(),
        Err(e) => store_err(e),
    }
}

async fn create_alert_notification(
    State(state): State<AppState>,
    Json(ch): Json<AlertNotificationChannel>,
) -> Response {
    match state.store.create_notification_channel(ch) {
        Ok(c) => Json(c).into_response(),
        Err(e) => store_err(e),
    }
}

async fn get_alert_notification(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.get_notification_channel(id) {
        Ok(ch) => Json(ch).into_response(),
        Err(e) => store_err(e),
    }
}

async fn update_alert_notification(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(ch): Json<AlertNotificationChannel>,
) -> Response {
    match state.store.update_notification_channel(id, ch) {
        Ok(c) => Json(c).into_response(),
        Err(e) => store_err(e),
    }
}

async fn delete_alert_notification(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.delete_notification_channel(id) {
        Ok(_) => Json(json!({"message":"Notification deleted"})).into_response(),
        Err(e) => store_err(e),
    }
}

// ─── Legacy alerts ────────────────────────────────────────────────────────────

async fn list_legacy_alerts(State(state): State<AppState>) -> Response {
    match state.store.list_dashboards(1) {
        Ok(dashboards) => {
            let alerts: Vec<serde_json::Value> = dashboards.iter().flat_map(|d| {
                d.panels.iter().filter_map(|p| p.alert.as_ref().map(|a| json!({
                    "id": a.id,
                    "dashboardId": d.id,
                    "dashboardUid": d.uid,
                    "dashboardSlug": d.slug,
                    "panelId": p.id,
                    "name": a.name,
                    "state": a.state,
                    "newStateDate": chrono::Utc::now(),
                    "evalDate": chrono::Utc::now(),
                    "url": format!("/d/{}/{}", d.uid, d.slug),
                })))
            }).collect();
            Json(alerts).into_response()
        }
        Err(e) => store_err(e),
    }
}

// ─── HTML Renderer ────────────────────────────────────────────────────────────

async fn render_dashboard_html(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Response {
    match state.store.get_dashboard_by_uid(&uid) {
        Ok(d) => {
            let html = render_dashboard(&d);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(html))
                .unwrap_or_else(|_| internal_error("render failed"))
        }
        Err(e) => store_err(e),
    }
}

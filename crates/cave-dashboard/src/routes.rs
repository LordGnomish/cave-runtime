//! Grafana API v1–compatible HTTP routes for CAVE Dashboard.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::{
    models::{
        Annotation, AnnotationType, AlertNotificationChannel, CreateAlertChannelRequest,
        CreateAnnotationRequest, CreateDataSourceRequest, CreateFolderRequest,
        CreatePlaylistRequest, CreateSnapshotRequest, CreateSnapshotResponse, DataSource,
        Folder, Playlist, SearchQuery, Snapshot, UpdateFolderRequest,
        UpsertDashboardRequest,
    },
    renderer::render_dashboard_html,
    store::DashboardStore,
    DashboardState,
};

type AppState = Arc<DashboardState>;
type ApiJson = Json<serde_json::Value>;

fn ok(v: serde_json::Value) -> ApiJson {
    Json(v)
}

fn err(status: StatusCode, msg: &str) -> (StatusCode, ApiJson) {
    (status, Json(json!({ "message": msg })))
}

fn not_found(msg: &str) -> (StatusCode, ApiJson) {
    err(StatusCode::NOT_FOUND, msg)
}

fn internal(msg: impl std::fmt::Display) -> (StatusCode, ApiJson) {
    err(StatusCode::INTERNAL_SERVER_ERROR, &msg.to_string())
}

fn lock(state: &DashboardState) -> std::sync::MutexGuard<'_, DashboardStore> {
    state.store.lock().expect("lock poisoned")
}

// ─── Router assembly ─────────────────────────────────────────────────────────

pub fn create_router(state: Arc<DashboardState>) -> Router {
    Router::new()
        // ── Health ──────────────────────────────────────────────────────────
        .route("/api/dashboard/health", get(health))
        // ── Dashboards ──────────────────────────────────────────────────────
        .route("/api/dashboards/db", post(upsert_dashboard))
        .route("/api/dashboards/uid/:uid", get(get_dashboard).delete(delete_dashboard))
        .route("/api/dashboards/home", get(home_dashboard))
        .route("/api/dashboards/import", post(import_dashboard))
        // ── Stars ───────────────────────────────────────────────────────────
        .route("/api/user/stars/dashboard/:uid", post(star_dashboard).delete(unstar_dashboard))
        // ── Search ──────────────────────────────────────────────────────────
        .route("/api/search", get(search))
        // ── Folders ─────────────────────────────────────────────────────────
        .route("/api/folders", get(list_folders).post(create_folder))
        .route("/api/folders/:uid", get(get_folder).put(update_folder).delete(delete_folder))
        // ── DataSources ─────────────────────────────────────────────────────
        .route("/api/datasources", get(list_datasources).post(create_datasource))
        .route(
            "/api/datasources/:id",
            get(get_datasource).put(update_datasource).delete(delete_datasource),
        )
        // ── Alert Notifications ─────────────────────────────────────────────
        .route(
            "/api/alert-notifications",
            get(list_alert_channels).post(create_alert_channel),
        )
        .route(
            "/api/alert-notifications/:id",
            get(get_alert_channel).put(update_alert_channel).delete(delete_alert_channel),
        )
        // ── Alerts (panel-level rules) ───────────────────────────────────────
        .route("/api/alerts", get(list_alerts))
        // ── Snapshots ───────────────────────────────────────────────────────
        .route("/api/snapshots", post(create_snapshot))
        .route("/api/snapshots/:key", get(get_snapshot).delete(delete_snapshot))
        // ── Annotations ─────────────────────────────────────────────────────
        .route("/api/annotations", get(list_annotations).post(create_annotation))
        .route("/api/annotations/:id", delete(delete_annotation))
        // ── Playlists ───────────────────────────────────────────────────────
        .route("/api/playlists", get(list_playlists).post(create_playlist))
        .route(
            "/api/playlists/:id",
            get(get_playlist).put(update_playlist).delete(delete_playlist),
        )
        // ── Renderer ────────────────────────────────────────────────────────
        .route("/api/dashboard/render/:uid", get(render_dashboard))
        .with_state(state)
}

// ─── Health ──────────────────────────────────────────────────────────────────

async fn health() -> ApiJson {
    ok(json!({
        "module": "cave-dashboard",
        "status": "ok",
        "upstream": "grafana (api-compat)",
    }))
}

// ─── Dashboards ──────────────────────────────────────────────────────────────

async fn upsert_dashboard(
    State(state): State<AppState>,
    Json(req): Json<UpsertDashboardRequest>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut dashboard =
        crate::provisioning::provision_from_json(&req.dashboard.to_string())
            .map_err(|e| internal(e))?;

    if let Some(fuid) = req.folder_uid {
        dashboard.folder_uid = Some(fuid);
    }

    let mut store = lock(&state);
    let saved = store.upsert_dashboard(dashboard);
    Ok(ok(json!({
        "id": saved.id,
        "uid": saved.uid,
        "url": saved.url(),
        "status": "success",
        "version": saved.version,
        "slug": saved.slug(),
    })))
}

async fn get_dashboard(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let store = lock(&state);
    let d = store.get_dashboard(&uid).ok_or_else(|| not_found("dashboard not found"))?;
    Ok(ok(json!({
        "dashboard": serde_json::to_value(d).unwrap_or_default(),
        "meta": {
            "isStarred": d.is_starred,
            "url": d.url(),
            "folderId": d.folder_uid,
            "slug": d.slug(),
            "version": d.version,
        }
    })))
}

async fn delete_dashboard(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.delete_dashboard(&uid) {
        Ok(ok(json!({ "title": uid, "message": "Dashboard deleted", "id": 0 })))
    } else {
        Err(not_found("dashboard not found"))
    }
}

async fn home_dashboard(State(state): State<AppState>) -> ApiJson {
    let store = lock(&state);
    let first = store.list_dashboards().into_iter().next();
    if let Some(d) = first {
        ok(json!({
            "dashboard": serde_json::to_value(d).unwrap_or_default(),
            "meta": { "isHome": true }
        }))
    } else {
        ok(json!({ "dashboard": null, "meta": { "isHome": true } }))
    }
}

async fn import_dashboard(
    State(state): State<AppState>,
    Json(req): Json<UpsertDashboardRequest>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    // Delegate to upsert with overwrite.
    let mut dashboard =
        crate::provisioning::provision_from_json(&req.dashboard.to_string())
            .map_err(|e| internal(e))?;
    if let Some(fuid) = req.folder_uid {
        dashboard.folder_uid = Some(fuid);
    }
    let mut store = lock(&state);
    let saved = store.upsert_dashboard(dashboard);
    Ok(ok(json!({
        "imported": true,
        "uid": saved.uid,
        "title": saved.title,
    })))
}

// ─── Stars ───────────────────────────────────────────────────────────────────

async fn star_dashboard(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.star_dashboard(&uid) {
        Ok(ok(json!({ "message": "Dashboard starred!" })))
    } else {
        Err(not_found("dashboard not found"))
    }
}

async fn unstar_dashboard(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> ApiJson {
    let mut store = lock(&state);
    store.unstar_dashboard(&uid);
    ok(json!({ "message": "Dashboard unstarred!" }))
}

// ─── Search ──────────────────────────────────────────────────────────────────

async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> ApiJson {
    let store = lock(&state);
    let results = store.search_dashboards(
        params.query.as_deref(),
        params.tag.as_deref(),
        params.folder_uid.as_deref(),
        params.starred,
    );
    let limit = params.limit.unwrap_or(1000);
    let items: Vec<serde_json::Value> = results
        .into_iter()
        .take(limit)
        .map(|d| {
            json!({
                "id": d.id,
                "uid": d.uid,
                "title": d.title,
                "url": d.url(),
                "type": "dash-db",
                "tags": d.tags,
                "isStarred": d.is_starred,
                "folderUid": d.folder_uid,
            })
        })
        .collect();
    ok(json!(items))
}

// ─── Folders ─────────────────────────────────────────────────────────────────

async fn list_folders(State(state): State<AppState>) -> ApiJson {
    let store = lock(&state);
    let items: Vec<serde_json::Value> = store
        .list_folders()
        .into_iter()
        .map(|f| serde_json::to_value(f).unwrap_or_default())
        .collect();
    ok(json!(items))
}

async fn create_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> ApiJson {
    let now = Utc::now();
    let uid = req.uid.unwrap_or_else(|| Uuid::new_v4().to_string());
    let url = format!("/dashboards/f/{}/{}", uid, req.title.to_lowercase().replace(' ', "-"));
    let folder = Folder {
        id: 0,
        uid: uid.clone(),
        title: req.title,
        url,
        created_at: now,
        updated_at: now,
    };
    let mut store = lock(&state);
    let saved = store.create_folder(folder);
    ok(serde_json::to_value(&saved).unwrap_or_default())
}

async fn get_folder(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let store = lock(&state);
    let f = store.get_folder(&uid).ok_or_else(|| not_found("folder not found"))?;
    Ok(ok(serde_json::to_value(f).unwrap_or_default()))
}

async fn update_folder(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(req): Json<UpdateFolderRequest>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    let f = store
        .update_folder(&uid, req.title)
        .ok_or_else(|| not_found("folder not found"))?;
    Ok(ok(serde_json::to_value(&f).unwrap_or_default()))
}

async fn delete_folder(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.delete_folder(&uid) {
        Ok(ok(json!({ "message": "Folder deleted", "uid": uid })))
    } else {
        Err(not_found("folder not found"))
    }
}

// ─── DataSources ─────────────────────────────────────────────────────────────

async fn list_datasources(State(state): State<AppState>) -> ApiJson {
    let store = lock(&state);
    let items: Vec<serde_json::Value> = store
        .list_datasources()
        .into_iter()
        .map(|ds| serde_json::to_value(ds).unwrap_or_default())
        .collect();
    ok(json!(items))
}

async fn create_datasource(
    State(state): State<AppState>,
    Json(req): Json<CreateDataSourceRequest>,
) -> ApiJson {
    let now = Utc::now();
    let ds = DataSource {
        id: 0,
        uid: Uuid::new_v4().to_string(),
        name: req.name,
        datasource_type: req.datasource_type,
        url: req.url,
        access: req.access.unwrap_or_default(),
        is_default: req.is_default.unwrap_or(false),
        basic_auth: false,
        json_data: req.json_data.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    let mut store = lock(&state);
    let saved = store.create_datasource(ds);
    ok(json!({ "datasource": serde_json::to_value(&saved).unwrap_or_default(), "id": saved.id, "message": "Datasource added", "name": saved.name }))
}

async fn get_datasource(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let store = lock(&state);
    let ds = store.get_datasource(id).ok_or_else(|| not_found("datasource not found"))?;
    Ok(ok(serde_json::to_value(ds).unwrap_or_default()))
}

async fn update_datasource(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Json(req): Json<CreateDataSourceRequest>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let now = Utc::now();
    let ds = DataSource {
        id,
        uid: Uuid::new_v4().to_string(),
        name: req.name,
        datasource_type: req.datasource_type,
        url: req.url,
        access: req.access.unwrap_or_default(),
        is_default: req.is_default.unwrap_or(false),
        basic_auth: false,
        json_data: req.json_data.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };
    let mut store = lock(&state);
    let updated =
        store.update_datasource(id, ds).ok_or_else(|| not_found("datasource not found"))?;
    Ok(ok(json!({ "datasource": serde_json::to_value(&updated).unwrap_or_default(), "id": updated.id, "message": "Datasource updated", "name": updated.name })))
}

async fn delete_datasource(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.delete_datasource(id) {
        Ok(ok(json!({ "message": "Data source deleted" })))
    } else {
        Err(not_found("datasource not found"))
    }
}

// ─── Alert Channels ───────────────────────────────────────────────────────────

async fn list_alert_channels(State(state): State<AppState>) -> ApiJson {
    let store = lock(&state);
    let items: Vec<serde_json::Value> = store
        .list_channels()
        .into_iter()
        .map(|c| serde_json::to_value(c).unwrap_or_default())
        .collect();
    ok(json!(items))
}

async fn create_alert_channel(
    State(state): State<AppState>,
    Json(req): Json<CreateAlertChannelRequest>,
) -> ApiJson {
    let now = Utc::now();
    let ch = AlertNotificationChannel {
        id: 0,
        uid: Uuid::new_v4().to_string(),
        name: req.name,
        channel_type: req.channel_type,
        settings: req.settings.unwrap_or_default(),
        is_default: req.is_default.unwrap_or(false),
        send_reminder: req.send_reminder.unwrap_or(false),
        frequency: req.frequency.unwrap_or_else(|| "15m".to_string()),
        created_at: now,
        updated_at: now,
    };
    let mut store = lock(&state);
    let saved = store.create_channel(ch);
    ok(serde_json::to_value(&saved).unwrap_or_default())
}

async fn get_alert_channel(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let store = lock(&state);
    let ch = store.get_channel(id).ok_or_else(|| not_found("alert channel not found"))?;
    Ok(ok(serde_json::to_value(ch).unwrap_or_default()))
}

async fn update_alert_channel(
    State(state): State<AppState>,
    Path(id): Path<u32>,
    Json(req): Json<CreateAlertChannelRequest>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let now = Utc::now();
    let ch = AlertNotificationChannel {
        id,
        uid: Uuid::new_v4().to_string(),
        name: req.name,
        channel_type: req.channel_type,
        settings: req.settings.unwrap_or_default(),
        is_default: req.is_default.unwrap_or(false),
        send_reminder: req.send_reminder.unwrap_or(false),
        frequency: req.frequency.unwrap_or_else(|| "15m".to_string()),
        created_at: now,
        updated_at: now,
    };
    let mut store = lock(&state);
    let updated =
        store.update_channel(id, ch).ok_or_else(|| not_found("alert channel not found"))?;
    Ok(ok(serde_json::to_value(&updated).unwrap_or_default()))
}

async fn delete_alert_channel(
    State(state): State<AppState>,
    Path(id): Path<u32>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.delete_channel(id) {
        Ok(ok(json!({ "message": "Notification deleted" })))
    } else {
        Err(not_found("alert channel not found"))
    }
}

// ─── Alerts (panel rules) ─────────────────────────────────────────────────────

async fn list_alerts(State(state): State<AppState>) -> ApiJson {
    let store = lock(&state);
    let rules = store.list_alert_rules();
    let items: Vec<serde_json::Value> =
        rules.iter().map(|r| serde_json::to_value(r).unwrap_or_default()).collect();
    ok(json!(items))
}

// ─── Snapshots ───────────────────────────────────────────────────────────────

async fn create_snapshot(
    State(state): State<AppState>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let now = Utc::now();
    let key = req.key.unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', ""));
    let delete_key =
        req.delete_key.unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', ""));
    let expires = req.expires.map(|secs| {
        now + chrono::Duration::seconds(secs)
    });
    let name = req.name.unwrap_or_else(|| "Snapshot".to_string());
    let snap = Snapshot {
        id: 0,
        key: key.clone(),
        delete_key: delete_key.clone(),
        name,
        dashboard: req.dashboard,
        expires,
        created_at: now,
        external: req.external.unwrap_or(false),
        external_url: None,
    };
    let mut store = lock(&state);
    let saved = store.create_snapshot(snap);
    let resp = CreateSnapshotResponse {
        key: saved.key.clone(),
        delete_key: saved.delete_key.clone(),
        url: format!("/dashboard/snapshot/{}", saved.key),
        delete_url: format!("/api/snapshots-delete/{}", saved.delete_key),
    };
    Ok(ok(serde_json::to_value(&resp).unwrap_or_default()))
}

async fn get_snapshot(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let store = lock(&state);
    let snap = store.get_snapshot(&key).ok_or_else(|| not_found("snapshot not found"))?;
    if snap.is_expired() {
        return Err(err(StatusCode::GONE, "snapshot has expired"));
    }
    Ok(ok(serde_json::to_value(snap).unwrap_or_default()))
}

async fn delete_snapshot(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.delete_snapshot(&key) {
        Ok(ok(json!({ "message": "Snapshot deleted" })))
    } else {
        Err(not_found("snapshot not found"))
    }
}

// ─── Annotations ─────────────────────────────────────────────────────────────

async fn list_annotations(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> ApiJson {
    let dashboard_uid = params.get("dashboardUID").map(String::as_str);
    let store = lock(&state);
    let items: Vec<serde_json::Value> = store
        .list_annotations(dashboard_uid)
        .into_iter()
        .map(|a| serde_json::to_value(a).unwrap_or_default())
        .collect();
    ok(json!(items))
}

async fn create_annotation(
    State(state): State<AppState>,
    Json(req): Json<CreateAnnotationRequest>,
) -> ApiJson {
    let ann = Annotation {
        id: 0,
        dashboard_uid: req.dashboard_uid,
        panel_id: req.panel_id,
        time: req.time.unwrap_or_else(Utc::now),
        time_end: req.time_end,
        tags: req.tags.unwrap_or_default(),
        text: req.text,
        annotation_type: AnnotationType::Manual,
    };
    let mut store = lock(&state);
    let saved = store.create_annotation(ann);
    ok(json!({ "id": saved.id, "message": "Annotation added" }))
}

async fn delete_annotation(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.delete_annotation(id) {
        Ok(ok(json!({ "message": "Annotation deleted" })))
    } else {
        Err(not_found("annotation not found"))
    }
}

// ─── Playlists ───────────────────────────────────────────────────────────────

async fn list_playlists(State(state): State<AppState>) -> ApiJson {
    let store = lock(&state);
    let items: Vec<serde_json::Value> = store
        .list_playlists()
        .into_iter()
        .map(|p| serde_json::to_value(p).unwrap_or_default())
        .collect();
    ok(json!(items))
}

async fn create_playlist(
    State(state): State<AppState>,
    Json(req): Json<CreatePlaylistRequest>,
) -> ApiJson {
    let now = Utc::now();
    let playlist = Playlist {
        id: Uuid::new_v4().to_string(),
        name: req.name,
        interval: req.interval,
        items: req.items,
        created_at: now,
        updated_at: now,
    };
    let mut store = lock(&state);
    let saved = store.create_playlist(playlist);
    ok(serde_json::to_value(&saved).unwrap_or_default())
}

async fn get_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let store = lock(&state);
    let p = store.get_playlist(&id).ok_or_else(|| not_found("playlist not found"))?;
    Ok(ok(serde_json::to_value(p).unwrap_or_default()))
}

async fn update_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreatePlaylistRequest>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let now = Utc::now();
    let playlist = Playlist {
        id: id.clone(),
        name: req.name,
        interval: req.interval,
        items: req.items,
        created_at: now,
        updated_at: now,
    };
    let mut store = lock(&state);
    let updated =
        store.update_playlist(&id, playlist).ok_or_else(|| not_found("playlist not found"))?;
    Ok(ok(serde_json::to_value(&updated).unwrap_or_default()))
}

async fn delete_playlist(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<ApiJson, (StatusCode, ApiJson)> {
    let mut store = lock(&state);
    if store.delete_playlist(&id) {
        Ok(ok(json!({ "message": "Playlist deleted" })))
    } else {
        Err(not_found("playlist not found"))
    }
}

// ─── Renderer ────────────────────────────────────────────────────────────────

async fn render_dashboard(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> Result<axum::response::Response<String>, (StatusCode, ApiJson)> {
    let store = lock(&state);
    let d = store.get_dashboard(&uid).ok_or_else(|| not_found("dashboard not found"))?;
    let html = render_dashboard_html(d);
    drop(store);
    Ok(axum::response::Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .body(html)
        .unwrap())
}

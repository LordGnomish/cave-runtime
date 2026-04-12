<<<<<<< HEAD
//! HTTP routes for cave-cache.

use crate::cache::PipelineOp;
use crate::models::{CacheStats, PubSubChannel};
use crate::CacheState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn create_router(state: Arc<CacheState>) -> Router {
    Router::new()
        .route("/api/v1/cache/keys", get(list_keys).post(set_key))
        .route("/api/v1/cache/keys/:key", get(get_key).delete(delete_key))
        .route("/api/v1/cache/keys/:key/expire", post(expire_key))
        .route("/api/v1/cache/keys/:key/incr", post(incr_key))
        .route("/api/v1/cache/keys/:key/decr", post(decr_key))
        .route("/api/v1/cache/pipeline", post(pipeline))
        .route("/api/v1/cache/stats", get(get_stats))
        .route("/api/v1/cache/pubsub", get(list_channels))
        .route("/api/v1/cache/pubsub/publish", post(publish))
        .route("/api/v1/cache/pubsub/:channel", get(subscribe))
        .with_state(state)
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct KeysQuery {
    pattern: Option<String>,
}

#[derive(Deserialize)]
struct SetKeyRequest {
    key: String,
    value: serde_json::Value,
    ttl: Option<u64>,
    tags: Option<Vec<String>>,
=======
use std::sync::Arc;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json,
};
use serde::{Deserialize, Serialize};
use crate::engine::CacheEngine;

pub type CacheState = Arc<CacheEngine>;

pub fn cache_router(state: CacheState) -> Router {
    Router::new()
        .route("/api/cache/health", get(health))
        .route("/api/cache/info", get(info))
        .route("/api/cache/get/:key", get(get_key))
        .route("/api/cache/set", post(set_key))
        .route("/api/cache/del/:key", delete(del_key))
        .route("/api/cache/type/:key", get(type_of_key))
        .route("/api/cache/ttl/:key", get(ttl_key))
        .route("/api/cache/expire", post(expire_key))
        .route("/api/cache/lpush", post(lpush_key))
        .route("/api/cache/lrange/:key", get(lrange_key))
        .route("/api/cache/hset", post(hset_key))
        .route("/api/cache/hgetall/:key", get(hgetall_key))
        .route("/api/cache/zadd", post(zadd_key))
        .route("/api/cache/zrange/:key", get(zrange_key))
        .route("/api/cache/publish", post(publish_msg))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok", "service": "cave-cache"}))
}

async fn info(State(engine): State<CacheState>) -> impl IntoResponse {
    let count = {
        let store = engine.store.lock().unwrap();
        store.len()
    };
    Json(serde_json::json!({
        "keys": count,
        "service": "cave-cache",
    }))
}

async fn get_key(
    State(engine): State<CacheState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match engine.get(&key) {
        Ok(Some(v)) => {
            let s = String::from_utf8(v).unwrap_or_default();
            (StatusCode::OK, Json(serde_json::json!({"value": s}))).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "not found"}))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct SetRequest {
    key: String,
    value: String,
    ttl_secs: Option<u64>,
}

async fn set_key(
    State(engine): State<CacheState>,
    Json(req): Json<SetRequest>,
) -> impl IntoResponse {
    let ex = req.ttl_secs.map(std::time::Duration::from_secs);
    match engine.set(&req.key, req.value.into_bytes(), ex) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn del_key(
    State(engine): State<CacheState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let n = engine.del(&[key.as_str()]);
    Json(serde_json::json!({"deleted": n}))
}

async fn type_of_key(
    State(engine): State<CacheState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match engine.type_of(&key) {
        Some(t) => (StatusCode::OK, Json(serde_json::json!({"type": t}))).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"type": "none"}))).into_response(),
    }
}

async fn ttl_key(
    State(engine): State<CacheState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match engine.ttl(&key) {
        Ok(t) => Json(serde_json::json!({"ttl": t})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
>>>>>>> claude/dazzling-tesla
}

#[derive(Deserialize)]
struct ExpireRequest {
<<<<<<< HEAD
    ttl: u64,
}

#[derive(Deserialize)]
struct IncrRequest {
    by: Option<i64>,
}

#[derive(Deserialize)]
struct PipelineRequest {
    operations: Vec<PipelineOp>,
}

#[derive(Deserialize)]
struct PublishRequest {
    channel: String,
    payload: serde_json::Value,
}

#[derive(Serialize)]
struct KeyListResponse {
    keys: Vec<String>,
    count: usize,
}

// ── Error helpers ─────────────────────────────────────────────────────────────

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err_not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": msg })))
}

fn err_bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg })))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_keys(
    Query(q): Query<KeysQuery>,
    State(state): State<Arc<CacheState>>,
) -> Json<KeyListResponse> {
    let pattern = q.pattern.as_deref().unwrap_or("*");
    let store = state.store.lock().unwrap();
    let keys = store.keys_matching(pattern);
    let count = keys.len();
    Json(KeyListResponse { keys, count })
}

async fn get_key(
    Path(key): Path<String>,
    State(state): State<Arc<CacheState>>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    match store.get(&key) {
        Some(entry) => Ok(Json(serde_json::to_value(&entry).unwrap_or_default())),
        None => Err(err_not_found(&format!("key '{key}' not found"))),
    }
}

async fn set_key(
    State(state): State<Arc<CacheState>>,
    Json(req): Json<SetKeyRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    store.set(req.key.clone(), req.value, req.ttl, req.tags.unwrap_or_default());
    Json(serde_json::json!({ "ok": true, "key": req.key }))
}

async fn delete_key(
    Path(key): Path<String>,
    State(state): State<Arc<CacheState>>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let deleted = store.delete(&key);
    Json(serde_json::json!({ "deleted": deleted, "key": key }))
}

async fn expire_key(
    Path(key): Path<String>,
    State(state): State<Arc<CacheState>>,
    Json(req): Json<ExpireRequest>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    if store.expire(&key, req.ttl) {
        Ok(Json(serde_json::json!({ "ok": true, "key": key, "ttl": req.ttl })))
    } else {
        Err(err_not_found(&format!("key '{key}' not found")))
    }
}

async fn incr_key(
    Path(key): Path<String>,
    State(state): State<Arc<CacheState>>,
    Json(req): Json<IncrRequest>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    match store.incr(&key, req.by.unwrap_or(1)) {
        Ok(val) => Ok(Json(serde_json::json!({ "key": key, "value": val }))),
        Err(e) => Err(err_bad_request(&e)),
    }
}

async fn decr_key(
    Path(key): Path<String>,
    State(state): State<Arc<CacheState>>,
    Json(req): Json<IncrRequest>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    match store.incr(&key, -req.by.unwrap_or(1)) {
        Ok(val) => Ok(Json(serde_json::json!({ "key": key, "value": val }))),
        Err(e) => Err(err_bad_request(&e)),
    }
}

async fn pipeline(
    State(state): State<Arc<CacheState>>,
    Json(req): Json<PipelineRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let results = store.pipeline(req.operations);
    Json(serde_json::json!({ "results": results }))
}

async fn get_stats(State(state): State<Arc<CacheState>>) -> Json<CacheStats> {
    let store = state.store.lock().unwrap();
    Json(store.stats.clone())
}

async fn list_channels(State(state): State<Arc<CacheState>>) -> Json<Vec<PubSubChannel>> {
    let store = state.store.lock().unwrap();
    let channels = store
        .channels
        .iter()
        .map(|(name, ch)| PubSubChannel {
            name: name.clone(),
            message_count: ch.message_count,
            recent_messages: ch.messages.iter().cloned().collect(),
        })
        .collect();
    Json(channels)
}

async fn publish(
    State(state): State<Arc<CacheState>>,
    Json(req): Json<PublishRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().unwrap();
    let count = store.publish(req.channel.clone(), req.payload);
    Json(serde_json::json!({ "channel": req.channel, "message_count": count }))
}

/// Poll recent messages for a channel (up to the last 100).
async fn subscribe(
    Path(channel): Path<String>,
    State(state): State<Arc<CacheState>>,
) -> Json<PubSubChannel> {
    let store = state.store.lock().unwrap();
    match store.channels.get(&channel) {
        Some(ch) => Json(PubSubChannel {
            name: channel,
            message_count: ch.message_count,
            recent_messages: ch.messages.iter().cloned().collect(),
        }),
        None => Json(PubSubChannel {
            name: channel,
            message_count: 0,
            recent_messages: vec![],
        }),
    }
}
=======
    key: String,
    secs: u64,
}

async fn expire_key(
    State(engine): State<CacheState>,
    Json(req): Json<ExpireRequest>,
) -> impl IntoResponse {
    match engine.expire(&req.key, req.secs) {
        Ok(ok) => Json(serde_json::json!({"ok": ok})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct LpushRequest {
    key: String,
    values: Vec<String>,
}

async fn lpush_key(
    State(engine): State<CacheState>,
    Json(req): Json<LpushRequest>,
) -> impl IntoResponse {
    let values: Vec<Vec<u8>> = req.values.into_iter().map(|v| v.into_bytes()).collect();
    match engine.lpush(&req.key, &values) {
        Ok(n) => Json(serde_json::json!({"length": n})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct LrangeQuery {
    start: Option<i64>,
    stop: Option<i64>,
}

async fn lrange_key(
    State(engine): State<CacheState>,
    Path(key): Path<String>,
    Query(q): Query<LrangeQuery>,
) -> impl IntoResponse {
    let start = q.start.unwrap_or(0);
    let stop = q.stop.unwrap_or(-1);
    match engine.lrange(&key, start, stop) {
        Ok(items) => {
            let strings: Vec<String> = items.into_iter()
                .map(|v| String::from_utf8(v).unwrap_or_default())
                .collect();
            Json(serde_json::json!({"items": strings})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct HsetRequest {
    key: String,
    fields: std::collections::HashMap<String, String>,
}

async fn hset_key(
    State(engine): State<CacheState>,
    Json(req): Json<HsetRequest>,
) -> impl IntoResponse {
    let fields: Vec<(Vec<u8>, Vec<u8>)> = req.fields
        .into_iter()
        .map(|(k, v)| (k.into_bytes(), v.into_bytes()))
        .collect();
    let field_refs: Vec<(&[u8], Vec<u8>)> = fields.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect();
    match engine.hset(&req.key, &field_refs) {
        Ok(n) => Json(serde_json::json!({"added": n})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn hgetall_key(
    State(engine): State<CacheState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match engine.hgetall(&key) {
        Ok(pairs) => {
            let map: std::collections::HashMap<String, String> = pairs
                .into_iter()
                .map(|(k, v)| (
                    String::from_utf8(k).unwrap_or_default(),
                    String::from_utf8(v).unwrap_or_default(),
                ))
                .collect();
            Json(serde_json::json!({"fields": map})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct ZaddMember {
    score: f64,
    member: String,
}

#[derive(Deserialize)]
struct ZaddRequest {
    key: String,
    members: Vec<ZaddMember>,
}

async fn zadd_key(
    State(engine): State<CacheState>,
    Json(req): Json<ZaddRequest>,
) -> impl IntoResponse {
    let members: Vec<(f64, Vec<u8>)> = req.members
        .into_iter()
        .map(|m| (m.score, m.member.into_bytes()))
        .collect();
    match engine.zadd(&req.key, &members) {
        Ok(n) => Json(serde_json::json!({"added": n})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct ZrangeQuery {
    start: Option<i64>,
    stop: Option<i64>,
}

async fn zrange_key(
    State(engine): State<CacheState>,
    Path(key): Path<String>,
    Query(q): Query<ZrangeQuery>,
) -> impl IntoResponse {
    let start = q.start.unwrap_or(0);
    let stop = q.stop.unwrap_or(-1);
    match engine.zrange(&key, start, stop, false) {
        Ok(items) => {
            let strings: Vec<String> = items.into_iter()
                .map(|v| String::from_utf8(v).unwrap_or_default())
                .collect();
            Json(serde_json::json!({"members": strings})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize, Serialize)]
struct PublishRequest {
    channel: String,
    message: String,
}

async fn publish_msg(
    State(engine): State<CacheState>,
    Json(req): Json<PublishRequest>,
) -> impl IntoResponse {
    let n = engine.publish(&req.channel, req.message.into_bytes());
    Json(serde_json::json!({"receivers": n}))
}
>>>>>>> claude/dazzling-tesla

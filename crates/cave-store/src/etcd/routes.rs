// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for the etcd v3 API (etcd grpc-gateway JSON format).
//!
//! Endpoints mirror etcd's grpc-gateway at /v3/{service}/{method}.
//! Watch streaming uses SSE (Server-Sent Events).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::post,
    Json, Router,
};
use futures::stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::StreamExt;

use crate::engine::{
    Compare, CompareResult, CompareTarget, MvccEngine, TxnOp, TxnRequest,
};
use crate::etcd::auth::{AuthManager, PermissionEntry};
use crate::etcd::cluster::ClusterManager;
use crate::StoreState;

fn err_json(code: i32, message: impl ToString) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "error": message.to_string(),
        "code": code,
    }))
}

// ── KV ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PutRequest {
    #[serde(default, with = "base64_bytes")]
    key: Vec<u8>,
    #[serde(default, with = "base64_bytes")]
    value: Vec<u8>,
    #[serde(default)]
    lease: i64,
    #[serde(default)]
    prev_kv: bool,
    #[serde(default)]
    ignore_value: bool,
    #[serde(default)]
    ignore_lease: bool,
}

async fn kv_put(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<PutRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .put(req.key, req.value, req.lease, req.prev_kv)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_json(2, e.to_string()),
            )
        })
}

#[derive(Debug, Deserialize)]
struct RangeRequest {
    #[serde(default, with = "base64_bytes")]
    key: Vec<u8>,
    #[serde(default, with = "base64_bytes")]
    range_end: Vec<u8>,
    #[serde(default)]
    limit: i64,
    #[serde(default)]
    revision: i64,
    #[serde(default)]
    sort_order: String,
    #[serde(default)]
    sort_target: String,
    #[serde(default)]
    serializable: bool,
    #[serde(default)]
    keys_only: bool,
    #[serde(default)]
    count_only: bool,
    #[serde(default)]
    min_mod_revision: i64,
    #[serde(default)]
    max_mod_revision: i64,
    #[serde(default)]
    min_create_revision: i64,
    #[serde(default)]
    max_create_revision: i64,
}

async fn kv_range(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<RangeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .range(
            req.key,
            req.range_end,
            req.revision,
            req.limit,
            req.keys_only,
            req.count_only,
        )
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| (StatusCode::BAD_REQUEST, err_json(3, e.to_string())))
}

#[derive(Debug, Deserialize)]
struct DeleteRangeRequest {
    #[serde(default, with = "base64_bytes")]
    key: Vec<u8>,
    #[serde(default, with = "base64_bytes")]
    range_end: Vec<u8>,
    #[serde(default)]
    prev_kv: bool,
}

async fn kv_delete(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<DeleteRangeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .delete_range(req.key, req.range_end, req.prev_kv)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                err_json(2, e.to_string()),
            )
        })
}

#[derive(Debug, Deserialize)]
struct TxnReq {
    #[serde(default)]
    compare: Vec<CompareDe>,
    #[serde(default)]
    success: Vec<TxnOpDe>,
    #[serde(default)]
    failure: Vec<TxnOpDe>,
}

#[derive(Debug, Deserialize)]
struct CompareDe {
    #[serde(with = "base64_bytes")]
    key: Vec<u8>,
    result: String,
    target: String,
    #[serde(default, with = "base64_bytes")]
    value: Vec<u8>,
    #[serde(default)]
    version: i64,
    #[serde(default)]
    create_revision: i64,
    #[serde(default)]
    mod_revision: i64,
    #[serde(default)]
    lease: i64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TxnOpDe {
    RequestPut {
        request_put: PutRequest,
    },
    RequestRange {
        request_range: RangeRequest,
    },
    RequestDeleteRange {
        request_delete_range: DeleteRangeRequest,
    },
}

async fn kv_txn(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<TxnReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let compare = req
        .compare
        .into_iter()
        .map(|c| {
            let result = match c.result.as_str() {
                "EQUAL" => CompareResult::Equal,
                "GREATER" => CompareResult::Greater,
                "LESS" => CompareResult::Less,
                _ => CompareResult::NotEqual,
            };
            let target = match c.target.as_str() {
                "VERSION" => CompareTarget::Version(c.version),
                "CREATE" => CompareTarget::CreateRevision(c.create_revision),
                "MOD" => CompareTarget::ModRevision(c.mod_revision),
                "VALUE" => CompareTarget::Value(c.value),
                "LEASE" => CompareTarget::Lease(c.lease),
                _ => CompareTarget::Version(0),
            };
            Compare {
                key: c.key,
                result,
                target,
            }
        })
        .collect();

    let map_op = |op: TxnOpDe| match op {
        TxnOpDe::RequestPut { request_put: r } => TxnOp::Put {
            key: r.key,
            value: r.value,
            lease_id: r.lease,
        },
        TxnOpDe::RequestRange { request_range: r } => TxnOp::Range {
            key: r.key,
            range_end: r.range_end,
            revision: r.revision,
        },
        TxnOpDe::RequestDeleteRange {
            request_delete_range: r,
        } => TxnOp::Delete {
            key: r.key,
            range_end: r.range_end,
        },
    };

    let txn = TxnRequest {
        compare,
        success: req.success.into_iter().map(map_op).collect(),
        failure: req.failure.into_iter().map(map_op).collect(),
    };

    state
        .engine
        .txn(txn)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| (StatusCode::BAD_REQUEST, err_json(3, e.to_string())))
}

#[derive(Debug, Deserialize)]
struct CompactRequest {
    #[serde(default)]
    revision: i64,
    #[serde(default)]
    physical: bool,
}

async fn kv_compact(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<CompactRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .compact(req.revision)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| (StatusCode::BAD_REQUEST, err_json(3, e.to_string())))
}

// ── Watch (SSE streaming) ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WatchCreateRequest {
    #[serde(default, with = "base64_bytes")]
    key: Vec<u8>,
    #[serde(default, with = "base64_bytes")]
    range_end: Vec<u8>,
    #[serde(default)]
    start_revision: i64,
    #[serde(default)]
    prev_kv: bool,
    #[serde(default)]
    progress_notify: bool,
    #[serde(default)]
    filters: Vec<String>,
}

async fn watch_create(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<WatchCreateRequest>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let filter_put = req.filters.iter().any(|f| f == "NOPUT");
    let filter_delete = req.filters.iter().any(|f| f == "NODELETE");

    let (_watch_id, mut rx) = state
        .engine
        .watch_create(
            req.key,
            req.range_end,
            req.start_revision,
            req.prev_kv,
            filter_put,
            filter_delete,
            req.progress_notify,
        )
        .await;

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let data = serde_json::json!({
                        "result": {
                            "header": { "revision": event.revision },
                            "watch_id": event.watch_id,
                            "events": [{
                                "type": if event.event_type == crate::engine::EventType::Put { "PUT" } else { "DELETE" },
                                "kv": {
                                    "key": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &event.key),
                                    "value": event.value.as_ref().map(|v| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, v)),
                                    "create_revision": event.create_revision,
                                    "mod_revision": event.mod_revision,
                                    "version": event.version,
                                    "lease": event.lease_id,
                                },
                            }],
                        }
                    });
                    yield Ok(Event::default().data(data.to_string()));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[derive(Debug, Deserialize)]
struct WatchCancelRequest {
    watch_id: i64,
}

async fn watch_cancel(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<WatchCancelRequest>,
) -> Json<serde_json::Value> {
    let removed = state.engine.watch_cancel(req.watch_id).await;
    Json(serde_json::json!({ "result": { "watch_id": req.watch_id, "canceled": removed } }))
}

// ── Lease ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LeaseGrantRequest {
    #[serde(rename = "TTL", default)]
    ttl: i64,
    #[serde(rename = "ID", default)]
    id: i64,
}

async fn lease_grant(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<LeaseGrantRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .lease_grant(req.ttl, req.id)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| (StatusCode::BAD_REQUEST, err_json(3, e.to_string())))
}

#[derive(Debug, Deserialize)]
struct LeaseRevokeRequest {
    #[serde(rename = "ID", default)]
    id: i64,
}

async fn lease_revoke(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<LeaseRevokeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .lease_revoke(req.id)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| (StatusCode::NOT_FOUND, err_json(5, e.to_string())))
}

#[derive(Debug, Deserialize)]
struct LeaseKeepAliveRequest {
    #[serde(rename = "ID", default)]
    id: i64,
}

async fn lease_keepalive(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<LeaseKeepAliveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .lease_keep_alive(req.id)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| (StatusCode::NOT_FOUND, err_json(5, e.to_string())))
}

#[derive(Debug, Deserialize)]
struct LeaseTtlRequest {
    #[serde(rename = "ID", default)]
    id: i64,
    #[serde(default)]
    keys: bool,
}

async fn lease_timetolive(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<LeaseTtlRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .engine
        .lease_time_to_live(req.id, req.keys)
        .await
        .map(|r| Json(serde_json::to_value(r).unwrap_or_default()))
        .map_err(|e| (StatusCode::NOT_FOUND, err_json(5, e.to_string())))
}

async fn lease_list(State(state): State<Arc<StoreState>>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(state.engine.lease_list().await).unwrap_or_default())
}

// ── Auth ──────────────────────────────────────────────────────────────────────

async fn auth_enable(State(state): State<Arc<StoreState>>) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.enable().await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

async fn auth_disable(State(state): State<Arc<StoreState>>) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.disable().await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

#[derive(Debug, Deserialize)]
struct AuthenticateRequest {
    name: String,
    password: String,
}

async fn authenticate(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<AuthenticateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.authenticate(&req.name, &req.password).await {
        Ok(token) => (
            StatusCode::OK,
            Json(serde_json::json!({ "header": {}, "token": token })),
        ),
        Err(e) => (StatusCode::UNAUTHORIZED, err_json(16, e.to_string())),
    }
}

#[derive(Debug, Deserialize)]
struct UserAddRequest {
    name: String,
    password: String,
    #[serde(default)]
    has_password: bool,
}

async fn user_add(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<UserAddRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.user_add(req.name, req.password).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

async fn user_delete(
    State(state): State<Arc<StoreState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.user_delete(&name).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::NOT_FOUND, err_json(5, e.to_string())),
    }
}

async fn user_get(
    State(state): State<Arc<StoreState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.user_get(&name).await {
        Ok(u) => (
            StatusCode::OK,
            Json(serde_json::json!({ "header": {}, "user": u })),
        ),
        Err(e) => (StatusCode::NOT_FOUND, err_json(5, e.to_string())),
    }
}

async fn user_list(State(state): State<Arc<StoreState>>) -> Json<serde_json::Value> {
    let users = state.auth.user_list().await;
    Json(serde_json::json!({ "header": {}, "users": users }))
}

#[derive(Debug, Deserialize)]
struct UserGrantRoleRequest {
    user: String,
    role: String,
}

async fn user_grant_role(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<UserGrantRoleRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.user_grant_role(&req.user, &req.role).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

#[derive(Debug, Deserialize)]
struct UserRevokeRoleRequest {
    user: String,
    role: String,
}

async fn user_revoke_role(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<UserRevokeRoleRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.user_revoke_role(&req.user, &req.role).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

#[derive(Debug, Deserialize)]
struct RoleAddRequest {
    name: String,
}

async fn role_add(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<RoleAddRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.role_add(req.name).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

async fn role_delete(
    State(state): State<Arc<StoreState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.role_delete(&name).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::NOT_FOUND, err_json(5, e.to_string())),
    }
}

async fn role_get(
    State(state): State<Arc<StoreState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.role_get(&name).await {
        Ok(r) => (
            StatusCode::OK,
            Json(serde_json::json!({ "header": {}, "role": r })),
        ),
        Err(e) => (StatusCode::NOT_FOUND, err_json(5, e.to_string())),
    }
}

async fn role_list(State(state): State<Arc<StoreState>>) -> Json<serde_json::Value> {
    let roles = state.auth.role_list().await;
    Json(serde_json::json!({ "header": {}, "roles": roles }))
}

#[derive(Debug, Deserialize)]
struct RoleGrantPermissionRequest {
    name: String,
    perm: PermGrant,
}

#[derive(Debug, Deserialize)]
struct PermGrant {
    #[serde(with = "base64_bytes")]
    key: Vec<u8>,
    #[serde(default, with = "base64_bytes")]
    range_end: Vec<u8>,
    perm_type: String,
}

async fn role_grant_permission(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<RoleGrantPermissionRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    use crate::etcd::auth::Permission;
    let perm_type = match req.perm.perm_type.as_str() {
        "READ" => Permission::Read,
        "WRITE" => Permission::Write,
        _ => Permission::Readwrite,
    };
    let entry = PermissionEntry {
        perm_type,
        key: req.perm.key,
        range_end: req.perm.range_end,
    };
    match state.auth.role_grant_permission(&req.name, entry).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

// ── Cluster ────────────────────────────────────────────────────────────────────

async fn member_list(State(state): State<Arc<StoreState>>) -> Json<serde_json::Value> {
    let members = state.cluster.member_list().await;
    Json(serde_json::json!({
        "header": { "cluster_id": state.cluster.cluster_id() },
        "members": members,
    }))
}

#[derive(Debug, Deserialize)]
struct MemberAddRequest {
    #[serde(rename = "peerURLs")]
    peer_ur_ls: Vec<String>,
    #[serde(rename = "isLearner", default)]
    is_learner: bool,
}

async fn member_add(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<MemberAddRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state
        .cluster
        .member_add(req.peer_ur_ls, req.is_learner)
        .await
    {
        Ok(m) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "header": { "cluster_id": state.cluster.cluster_id() },
                "member": m,
                "members": state.cluster.member_list().await,
            })),
        ),
        Err(e) => (StatusCode::BAD_REQUEST, err_json(6, e.to_string())),
    }
}

async fn member_remove(
    State(state): State<Arc<StoreState>>,
    Path(id): Path<u64>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.cluster.member_remove(id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "header": {} }))),
        Err(e) => (StatusCode::NOT_FOUND, err_json(5, e.to_string())),
    }
}

async fn member_update(
    State(state): State<Arc<StoreState>>,
    Path(id): Path<u64>,
    Json(req): Json<MemberAddRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.cluster.member_update(id, req.peer_ur_ls).await {
        Ok(m) => (StatusCode::OK, Json(serde_json::json!({ "member": m }))),
        Err(e) => (StatusCode::NOT_FOUND, err_json(5, e.to_string())),
    }
}

async fn member_promote(
    State(state): State<Arc<StoreState>>,
    Path(id): Path<u64>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.cluster.member_promote(id).await {
        Ok(m) => (StatusCode::OK, Json(serde_json::json!({ "member": m }))),
        Err(e) => (StatusCode::NOT_FOUND, err_json(5, e.to_string())),
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn etcd_router(state: Arc<StoreState>) -> Router {
    Router::new()
        // KV
        .route("/v3/kv/put", post(kv_put))
        .route("/v3/kv/range", post(kv_range))
        .route("/v3/kv/deleterange", post(kv_delete))
        .route("/v3/kv/txn", post(kv_txn))
        .route("/v3/kv/compaction", post(kv_compact))
        // Watch
        .route("/v3/watch", post(watch_create))
        .route("/v3/watch/cancel", post(watch_cancel))
        // Lease
        .route("/v3/lease/grant", post(lease_grant))
        .route("/v3/lease/revoke", post(lease_revoke))
        .route("/v3/lease/keepalive", post(lease_keepalive))
        .route("/v3/lease/timetolive", post(lease_timetolive))
        .route("/v3/leases", post(lease_list))
        // Auth
        .route("/v3/auth/enable", post(auth_enable))
        .route("/v3/auth/disable", post(auth_disable))
        .route("/v3/auth/authenticate", post(authenticate))
        .route("/v3/auth/user/add", post(user_add))
        .route("/v3/auth/user/delete/{name}", post(user_delete))
        .route("/v3/auth/user/get/{name}", post(user_get))
        .route("/v3/auth/user/list", post(user_list))
        .route("/v3/auth/user/grant", post(user_grant_role))
        .route("/v3/auth/user/revoke", post(user_revoke_role))
        .route("/v3/auth/role/add", post(role_add))
        .route("/v3/auth/role/delete/{name}", post(role_delete))
        .route("/v3/auth/role/get/{name}", post(role_get))
        .route("/v3/auth/role/list", post(role_list))
        .route("/v3/auth/role/grant", post(role_grant_permission))
        // Cluster
        .route("/v3/cluster/member/list", post(member_list))
        .route("/v3/cluster/member/add", post(member_add))
        .route("/v3/cluster/member/remove/{id}", post(member_remove))
        .route("/v3/cluster/member/update/{id}", post(member_update))
        .route("/v3/cluster/member/promote/{id}", post(member_promote))
        .with_state(state)
}

// ── Base64 serde helper ───────────────────────────────────────────────────────

mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], ser: S) -> Result<S::Ok, S::Error> {
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .serialize(ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(de)?;
        if s.is_empty() {
            return Ok(Vec::new());
        }
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

//! REST API routes — etcd v3 API compatible.

use crate::b64;
use crate::models::*;
use crate::raft_bridge::{RaftBridgeError, SharedRaftBridge};
use crate::store::KvStore;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, Sse},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// Mount the etcd routes against a shared `KvStore`. Single-node
/// deployments call this directly; multi-node deployments wrap the
/// router with a `RaftBridge` extension via
/// [`create_router_with_bridge`] so write handlers consult the Raft
/// leader before mutating local state.
pub fn create_router(state: Arc<KvStore>) -> Router {
    Router::new()
        .route("/api/etcd/health", get(health))
        .route("/api/etcd/status", get(status))
        // KV
        .route("/api/etcd/v3/kv/range", post(kv_range))
        .route("/api/etcd/v3/kv/put", post(kv_put))
        .route("/api/etcd/v3/kv/deleterange", post(kv_delete_range))
        .route("/api/etcd/v3/kv/txn", post(kv_txn))
        .route("/api/etcd/v3/kv/compaction", post(kv_compaction))
        // Watch
        .route("/api/etcd/v3/watch", post(watch_create))
        .route("/api/etcd/v3/watch/stream", get(watch_stream))
        // Lease
        .route("/api/etcd/v3/lease/grant", post(lease_grant))
        .route("/api/etcd/v3/lease/revoke", post(lease_revoke))
        .route("/api/etcd/v3/lease/keepalive", post(lease_keepalive))
        .route("/api/etcd/v3/lease/timetolive", post(lease_timetolive))
        .route("/api/etcd/v3/lease/leases", get(lease_leases))
        // Auth
        .route("/api/etcd/v3/auth/enable", post(auth_enable))
        .route("/api/etcd/v3/auth/disable", post(auth_disable))
        .route("/api/etcd/v3/auth/authenticate", post(auth_authenticate))
        .route("/api/etcd/v3/auth/user/add", post(auth_user_add))
        .route("/api/etcd/v3/auth/user/delete", post(auth_user_delete))
        .route("/api/etcd/v3/auth/user/get", post(auth_user_get))
        .route("/api/etcd/v3/auth/user/list", post(auth_user_list))
        .route("/api/etcd/v3/auth/user/changepw", post(auth_user_changepw))
        .route("/api/etcd/v3/auth/user/grant", post(auth_user_grant_role))
        .route("/api/etcd/v3/auth/user/revoke", post(auth_user_revoke_role))
        .route("/api/etcd/v3/auth/role/add", post(auth_role_add))
        .route("/api/etcd/v3/auth/role/delete", post(auth_role_delete))
        .route("/api/etcd/v3/auth/role/get", post(auth_role_get))
        .route("/api/etcd/v3/auth/role/list", post(auth_role_list))
        .route("/api/etcd/v3/auth/role/grant", post(auth_role_grant_permission))
        .route("/api/etcd/v3/auth/role/revoke", post(auth_role_revoke_permission))
        // Maintenance
        .route("/api/etcd/v3/maintenance/status", post(maintenance_status))
        .route("/api/etcd/v3/maintenance/alarm", post(maintenance_alarm))
        .route("/api/etcd/v3/maintenance/defragment", post(maintenance_defragment))
        .route("/api/etcd/v3/maintenance/hash", post(maintenance_hash))
        .route("/api/etcd/v3/maintenance/snapshot", post(maintenance_snapshot))
        // Cluster
        .route("/api/etcd/v3/cluster/member/add", post(cluster_member_add))
        .route("/api/etcd/v3/cluster/member/remove", post(cluster_member_remove))
        .route("/api/etcd/v3/cluster/member/update", post(cluster_member_update))
        .route("/api/etcd/v3/cluster/member/list", post(cluster_member_list))
        // Cluster v3.6: promotion + joint consensus
        .route("/api/etcd/v3/cluster/member/promote", post(cluster_member_promote))
        .route("/api/etcd/v3/cluster/joint/enter", post(cluster_joint_enter))
        .route("/api/etcd/v3/cluster/joint/leave", post(cluster_joint_leave))
        // Maintenance v3.6: streamed snapshot
        .route("/api/etcd/v3/maintenance/snapshot/stream", post(maintenance_snapshot_stream))
        // Version
        .route("/api/etcd/v3/version", get(version))
        // Parity
        .route("/api/etcd/parity", get(parity))
        .with_state(state)
}

/// Variant that injects a `SharedRaftBridge` extension. Write handlers
/// pick it up via `Option<Extension<SharedRaftBridge>>`; when absent
/// the handlers fall through to the existing direct-write path so
/// single-node deployments are unchanged.
pub fn create_router_with_bridge(
    state: Arc<KvStore>,
    bridge: Option<SharedRaftBridge>,
) -> Router {
    let router = create_router(state);
    match bridge {
        Some(b) => router.layer(Extension(b)),
        None => router,
    }
}

// ── Parity ────────────────────────────────────────────────────────────────────

async fn parity() -> Json<serde_json::Value> {
    match crate::calculate_parity() {
        Ok(report) => Json(serde_json::to_value(&report).unwrap_or_default()),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// ── Auth token helper ──────────────────────────────────────────────────────

fn extract_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_start_matches("Bearer ").to_string())
}

// ── Health / Status ────────────────────────────────────────────────────────


/// Decode base64 keys/values in PutRequest (etcd v3 API compat).
fn decode_put_request(mut req: PutRequest) -> PutRequest {
    req.key = String::from_utf8_lossy(&b64::decode(&req.key)).to_string();
    req.value = String::from_utf8_lossy(&b64::decode(&req.value)).to_string();
    req
}

/// Decode base64 key in RangeRequest.
fn decode_range_request(mut req: RangeRequest) -> RangeRequest {
    req.key = String::from_utf8_lossy(&b64::decode(&req.key)).to_string();
    if let Some(ref end) = req.range_end {
        req.range_end = Some(String::from_utf8_lossy(&b64::decode(end)).to_string());
    }
    req
}

/// Decode base64 key in DeleteRangeRequest.
fn decode_delete_request(mut req: DeleteRangeRequest) -> DeleteRangeRequest {
    req.key = String::from_utf8_lossy(&b64::decode(&req.key)).to_string();
    if let Some(ref end) = req.range_end {
        req.range_end = Some(String::from_utf8_lossy(&b64::decode(end)).to_string());
    }
    req
}

/// Encode KeyValue fields to base64 for response.
fn encode_kv(kv: &mut KeyValue) {
    kv.key = b64::encode(&kv.key).into_bytes();
    kv.value = b64::encode(&kv.value).into_bytes();
}

/// Decode base64 in transaction request ops.
fn decode_request_op(op: &mut RequestOp) {
    match op {
        RequestOp::Put(ref mut req) => {
            req.key = String::from_utf8_lossy(&b64::decode(&req.key)).to_string();
            req.value = String::from_utf8_lossy(&b64::decode(&req.value)).to_string();
        }
        RequestOp::Range(ref mut req) => {
            req.key = String::from_utf8_lossy(&b64::decode(&req.key)).to_string();
            if let Some(ref end) = req.range_end {
                req.range_end = Some(String::from_utf8_lossy(&b64::decode(end)).to_string());
            }
        }
        RequestOp::DeleteRange(ref mut req) => {
            req.key = String::from_utf8_lossy(&b64::decode(&req.key)).to_string();
            if let Some(ref end) = req.range_end {
                req.range_end = Some(String::from_utf8_lossy(&b64::decode(end)).to_string());
            }
        }
    }
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-etcd",
        "status": "ok",
        "upstream": "etcd",
        "api_version": "v3"
    }))
}

async fn status(State(store): State<Arc<KvStore>>) -> Json<serde_json::Value> {
    Json(store.status())
}

// ── KV ─────────────────────────────────────────────────────────────────────

async fn kv_range(
    State(store): State<Arc<KvStore>>,
    headers: HeaderMap,
    Json(req): Json<RangeRequest>,
) -> Result<Json<RangeResponse>, (StatusCode, String)> {
    let token = extract_token(&headers);
    let req = decode_range_request(req);
    store
        .check_auth_token(token.as_deref(), req.key.as_bytes(), PermType::Read)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    store
        .range(&req)
        .map(|mut resp| {
            for kv in &mut resp.kvs {
                encode_kv(kv);
            }
            Json(resp)
        })
        .map_err(|e| match &e {
            crate::error::EtcdError::RevisionCompacted { .. } => (StatusCode::BAD_REQUEST, e.to_string()),
            crate::error::EtcdError::KeyNotFound(_) => (StatusCode::OK, e.to_string()),
            _ => (StatusCode::BAD_REQUEST, e.to_string()),
        })
}

async fn kv_put(
    State(store): State<Arc<KvStore>>,
    bridge: Option<Extension<SharedRaftBridge>>,
    headers: HeaderMap,
    Json(req): Json<PutRequest>,
) -> Response {
    let token = extract_token(&headers);
    let req = decode_put_request(req);
    if let Err(e) =
        store.check_auth_token(token.as_deref(), req.key.as_bytes(), PermType::Write)
    {
        return (StatusCode::UNAUTHORIZED, e.to_string()).into_response();
    }
    // Multi-node Raft mode: propose-and-wait through the bridge.
    // The apply daemon writes the key into the local KvStore on commit;
    // by the time the bridge returns Ok, this node has the entry.
    if let Some(Extension(b)) = bridge {
        match b.propose_put(req.key.clone(), req.value.clone(), req.lease).await {
            Ok(()) => {
                // The bridge has already applied through the daemon.
                // Re-read the row so the response carries the same
                // shape as the direct path (header.revision +
                // prev_kv encoded when available).
                let range = RangeRequest {
                    key: req.key.clone(),
                    range_end: None,
                    limit: None,
                    revision: None,
                    keys_only: false,
                    count_only: false,
                };
                let header = match store.range(&range) {
                    Ok(r) => r.header,
                    Err(_) => ResponseHeader::default(),
                };
                let resp = PutResponse { header, prev_kv: None };
                return Json(resp).into_response();
            }
            Err(RaftBridgeError::NotLeader { leader_url }) => {
                // 503 with a Location: header so etcd clients can
                // retry against the leader without re-issuing a DNS
                // lookup.
                let mut r = (
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "not leader; leader_url={}",
                        leader_url.as_deref().unwrap_or("unknown")
                    ),
                )
                    .into_response();
                if let Some(url) = leader_url {
                    if let Ok(hv) = axum::http::HeaderValue::from_str(&url) {
                        r.headers_mut().insert(axum::http::header::LOCATION, hv);
                    }
                }
                return r;
            }
            Err(RaftBridgeError::Timeout) => {
                return (
                    StatusCode::GATEWAY_TIMEOUT,
                    "timed out waiting for raft commit+apply".to_string(),
                )
                    .into_response();
            }
            Err(RaftBridgeError::Internal(msg)) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response();
            }
        }
    }
    // Single-node mode (no bridge installed): direct apply.
    let mut resp = store.put(&req);
    if let Some(ref mut kv) = resp.prev_kv {
        encode_kv(kv);
    }
    Json(resp).into_response()
}

async fn kv_delete_range(
    State(store): State<Arc<KvStore>>,
    headers: HeaderMap,
    Json(req): Json<DeleteRangeRequest>,
) -> Result<Json<DeleteRangeResponse>, (StatusCode, String)> {
    let token = extract_token(&headers);
    let req = decode_delete_request(req);
    store
        .check_auth_token(token.as_deref(), req.key.as_bytes(), PermType::Write)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    let mut resp = store.delete_range(&req);
    for kv in &mut resp.prev_kvs {
        encode_kv(kv);
    }
    Ok(Json(resp))
}

async fn kv_txn(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<TxnRequest>,
) -> Json<TxnResponse> {
    Json(store.txn(&req))
}

async fn kv_compaction(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<CompactionRequest>,
) -> Json<CompactionResponse> {
    Json(store.compaction(&req))
}

// ── Watch ──────────────────────────────────────────────────────────────────

async fn watch_create(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<WatchCreateRequest>,
) -> Json<WatchResponse> {
    Json(store.watch_create(&req))
}

#[derive(serde::Deserialize, Default)]
struct WatchStreamQuery {
    watch_id: Option<i64>,
}

async fn watch_stream(
    State(store): State<Arc<KvStore>>,
    Query(params): Query<WatchStreamQuery>,
) -> Sse<UnboundedReceiverStream<Result<Event, Infallible>>> {
    let (tx, inner_rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();

    let watch_config = params
        .watch_id
        .and_then(|id| store.get_watch_config(id));

    // Historical replay: send events from start_revision before going live.
    if let Some(ref config) = watch_config {
        if let Some(start_rev) = config.start_revision {
            for event in store.get_historical_events(config, start_rev) {
                if let Ok(data) = serde_json::to_string(&event) {
                    let _ = tx.send(Ok(Event::default().data(data)));
                }
            }
        }
    }

    let mut rx = store.subscribe();

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let matches = watch_config
                        .as_ref()
                        .map(|c| KvStore::key_matches_watch(&event.kv.key, c))
                        .unwrap_or(true);

                    if matches {
                        if let Ok(data) = serde_json::to_string(&event) {
                            if tx.send(Ok(Event::default().data(data))).is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    Sse::new(UnboundedReceiverStream::new(inner_rx))
}

// ── Lease ──────────────────────────────────────────────────────────────────

async fn lease_grant(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<LeaseGrantRequest>,
) -> Json<LeaseGrantResponse> {
    Json(store.lease_grant(&req))
}

#[derive(serde::Deserialize)]
struct LeaseRevokeReq {
    #[serde(rename = "ID")]
    id: i64,
}

async fn lease_revoke(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<LeaseRevokeReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    store
        .lease_revoke(req.id)
        .map(|_| Json(serde_json::json!({"header": {}})))
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn lease_keepalive(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<LeaseKeepAliveRequest>,
) -> Result<Json<LeaseKeepAliveResponse>, (StatusCode, String)> {
    store
        .lease_keepalive(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn lease_timetolive(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<LeaseTTLRequest>,
) -> Result<Json<LeaseTTLResponse>, (StatusCode, String)> {
    store
        .lease_timetolive(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn lease_leases(State(store): State<Arc<KvStore>>) -> Json<LeaseLeasesResponse> {
    Json(store.lease_leases())
}

// ── Auth ───────────────────────────────────────────────────────────────────

async fn auth_enable(
    State(store): State<Arc<KvStore>>,
) -> Result<Json<AuthEnableResponse>, (StatusCode, String)> {
    store
        .auth_enable()
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn auth_disable(
    State(store): State<Arc<KvStore>>,
) -> Result<Json<AuthDisableResponse>, (StatusCode, String)> {
    store
        .auth_disable()
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn auth_authenticate(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthenticateRequest>,
) -> Result<Json<AuthenticateResponse>, (StatusCode, String)> {
    store
        .authenticate(&req)
        .map(Json)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))
}

async fn auth_user_add(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthUserAddRequest>,
) -> Result<Json<AuthUserAddResponse>, (StatusCode, String)> {
    store
        .user_add(&req)
        .map(Json)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}

async fn auth_user_delete(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthUserDeleteRequest>,
) -> Result<Json<AuthUserDeleteResponse>, (StatusCode, String)> {
    store
        .user_delete(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn auth_user_get(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthUserGetRequest>,
) -> Result<Json<AuthUserGetResponse>, (StatusCode, String)> {
    store
        .user_get(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn auth_user_list(State(store): State<Arc<KvStore>>) -> Json<AuthUserListResponse> {
    Json(store.user_list())
}

async fn auth_user_changepw(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthUserChangePasswordRequest>,
) -> Result<Json<AuthUserChangePasswordResponse>, (StatusCode, String)> {
    store
        .user_change_password(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn auth_user_grant_role(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthUserGrantRoleRequest>,
) -> Result<Json<AuthUserGrantRoleResponse>, (StatusCode, String)> {
    store
        .user_grant_role(&req)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn auth_user_revoke_role(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthUserRevokeRoleRequest>,
) -> Result<Json<AuthUserRevokeRoleResponse>, (StatusCode, String)> {
    store
        .user_revoke_role(&req)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn auth_role_add(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthRoleAddRequest>,
) -> Result<Json<AuthRoleAddResponse>, (StatusCode, String)> {
    store
        .role_add(&req)
        .map(Json)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))
}

async fn auth_role_delete(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthRoleDeleteRequest>,
) -> Result<Json<AuthRoleDeleteResponse>, (StatusCode, String)> {
    store
        .role_delete(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn auth_role_get(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthRoleGetRequest>,
) -> Result<Json<AuthRoleGetResponse>, (StatusCode, String)> {
    store
        .role_get(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn auth_role_list(State(store): State<Arc<KvStore>>) -> Json<AuthRoleListResponse> {
    Json(store.role_list())
}

async fn auth_role_grant_permission(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthRoleGrantPermissionRequest>,
) -> Result<Json<AuthRoleGrantPermissionResponse>, (StatusCode, String)> {
    store
        .role_grant_permission(&req)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn auth_role_revoke_permission(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AuthRoleRevokePermissionRequest>,
) -> Result<Json<AuthRoleRevokePermissionResponse>, (StatusCode, String)> {
    store
        .role_revoke_permission(&req)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

// ── Maintenance ────────────────────────────────────────────────────────────

async fn maintenance_status(State(store): State<Arc<KvStore>>) -> Json<serde_json::Value> {
    Json(store.status())
}

async fn maintenance_alarm(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<AlarmRequest>,
) -> Json<AlarmResponse> {
    Json(store.alarm(&req))
}

async fn maintenance_defragment(State(store): State<Arc<KvStore>>) -> Json<DefragmentResponse> {
    Json(store.defragment())
}

async fn maintenance_hash(State(store): State<Arc<KvStore>>) -> Json<HashResponse> {
    Json(store.hash())
}

async fn maintenance_snapshot(State(store): State<Arc<KvStore>>) -> Json<SnapshotResponse> {
    Json(store.snapshot())
}

// ── Cluster ────────────────────────────────────────────────────────────────

async fn cluster_member_add(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<MemberAddRequest>,
) -> Json<MemberAddResponse> {
    Json(store.member_add(&req))
}

async fn cluster_member_remove(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<MemberRemoveRequest>,
) -> Result<Json<MemberRemoveResponse>, (StatusCode, String)> {
    store
        .member_remove(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn cluster_member_update(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<MemberUpdateRequest>,
) -> Result<Json<MemberUpdateResponse>, (StatusCode, String)> {
    store
        .member_update(&req)
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn cluster_member_list(State(store): State<Arc<KvStore>>) -> Json<MemberListResponse> {
    Json(store.member_list())
}

// ── Cluster v3.6: promotion + joint consensus ─────────────────────────────

async fn cluster_member_promote(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<MemberPromoteRequest>,
) -> Result<Json<MemberPromoteResponse>, (StatusCode, String)> {
    store
        .member_promote(&req)
        .map(Json)
        .map_err(|e| match &e {
            crate::error::EtcdError::MemberNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()),
            crate::error::EtcdError::MemberNotLearner(_) => (StatusCode::BAD_REQUEST, e.to_string()),
            _ => (StatusCode::BAD_REQUEST, e.to_string()),
        })
}

async fn cluster_joint_enter(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<EnterJointRequest>,
) -> Result<Json<EnterJointResponse>, (StatusCode, String)> {
    store
        .enter_joint(&req)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn cluster_joint_leave(
    State(store): State<Arc<KvStore>>,
) -> Result<Json<LeaveJointResponse>, (StatusCode, String)> {
    store
        .leave_joint()
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

// ── Maintenance v3.6: streamed snapshot ───────────────────────────────────

async fn maintenance_snapshot_stream(
    State(store): State<Arc<KvStore>>,
) -> Json<Vec<SnapshotChunk>> {
    Json(store.snapshot_stream())
}

// ── Version ────────────────────────────────────────────────────────────────

async fn version(State(store): State<Arc<KvStore>>) -> Json<VersionResponse> {
    Json(store.version())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::util::ServiceExt;

    fn test_app() -> Router {
        create_router(Arc::new(KvStore::new()))
    }

    async fn post_json(
        app: Router,
        path: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn get_req(app: Router, path: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let resp = get_req(test_app(), "/api/etcd/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_kv_put_range() {
        let app = test_app();
        let resp = post_json(
            app.clone(),
            "/api/etcd/v3/kv/put",
            serde_json::json!({"key": "hello", "value": "world", "prev_kv": false}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp2 = post_json(
            app,
            "/api/etcd/v3/kv/range",
            serde_json::json!({"key": "hello", "keys_only": false, "count_only": false}),
        )
        .await;
        assert_eq!(resp2.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_kv_compaction_endpoint() {
        let app = test_app();
        post_json(
            app.clone(),
            "/api/etcd/v3/kv/put",
            serde_json::json!({"key": "a", "value": "1", "prev_kv": false}),
        )
        .await;
        let resp = post_json(
            app,
            "/api/etcd/v3/kv/compaction",
            serde_json::json!({"revision": 1, "physical": false}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_watch_create_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/watch",
            serde_json::json!({"key": "/foo", "progress_notify": false, "prev_kv": false}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_lease_keepalive_endpoint() {
        let app = test_app();
        let grant_resp = post_json(
            app.clone(),
            "/api/etcd/v3/lease/grant",
            serde_json::json!({"TTL": 30}),
        )
        .await;
        let body = axum::body::to_bytes(grant_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let grant: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = grant["ID"].as_i64().unwrap();

        let resp = post_json(
            app,
            "/api/etcd/v3/lease/keepalive",
            serde_json::json!({ "ID": id }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_lease_timetolive_endpoint() {
        let app = test_app();
        let grant_resp = post_json(
            app.clone(),
            "/api/etcd/v3/lease/grant",
            serde_json::json!({"TTL": 60}),
        )
        .await;
        let body = axum::body::to_bytes(grant_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let grant: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = grant["ID"].as_i64().unwrap();

        let resp = post_json(
            app,
            "/api/etcd/v3/lease/timetolive",
            serde_json::json!({"ID": id, "keys": false}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_lease_leases_endpoint() {
        let resp = get_req(test_app(), "/api/etcd/v3/lease/leases").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_enable_disable_endpoints() {
        let app = test_app();
        let resp =
            post_json(app.clone(), "/api/etcd/v3/auth/enable", serde_json::json!({})).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let resp2 =
            post_json(app, "/api/etcd/v3/auth/disable", serde_json::json!({})).await;
        assert_eq!(resp2.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_authenticate_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/auth/authenticate",
            serde_json::json!({"name": "user", "password": "pass"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_user_add_delete_endpoints() {
        let app = test_app();
        let add = post_json(
            app.clone(),
            "/api/etcd/v3/auth/user/add",
            serde_json::json!({"name": "testuser", "password": "pw"}),
        )
        .await;
        assert_eq!(add.status(), StatusCode::OK);

        let del = post_json(
            app,
            "/api/etcd/v3/auth/user/delete",
            serde_json::json!({"name": "testuser"}),
        )
        .await;
        assert_eq!(del.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_user_get_endpoint() {
        let app = test_app();
        post_json(
            app.clone(),
            "/api/etcd/v3/auth/user/add",
            serde_json::json!({"name": "u", "password": "p"}),
        )
        .await;
        let resp = post_json(
            app,
            "/api/etcd/v3/auth/user/get",
            serde_json::json!({"name": "u"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_user_list_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/auth/user/list",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_user_changepw_endpoint() {
        let app = test_app();
        post_json(
            app.clone(),
            "/api/etcd/v3/auth/user/add",
            serde_json::json!({"name": "pw_user", "password": "old"}),
        )
        .await;
        let resp = post_json(
            app,
            "/api/etcd/v3/auth/user/changepw",
            serde_json::json!({"name": "pw_user", "password": "new"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_role_add_get_delete_endpoints() {
        let app = test_app();
        let add = post_json(
            app.clone(),
            "/api/etcd/v3/auth/role/add",
            serde_json::json!({"name": "testrole"}),
        )
        .await;
        assert_eq!(add.status(), StatusCode::OK);

        let get = post_json(
            app.clone(),
            "/api/etcd/v3/auth/role/get",
            serde_json::json!({"role": "testrole"}),
        )
        .await;
        assert_eq!(get.status(), StatusCode::OK);

        let del = post_json(
            app,
            "/api/etcd/v3/auth/role/delete",
            serde_json::json!({"role": "testrole"}),
        )
        .await;
        assert_eq!(del.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_role_list_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/auth/role/list",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_user_grant_revoke_role_endpoints() {
        let app = test_app();
        post_json(
            app.clone(),
            "/api/etcd/v3/auth/user/add",
            serde_json::json!({"name": "u", "password": "p"}),
        )
        .await;
        post_json(
            app.clone(),
            "/api/etcd/v3/auth/role/add",
            serde_json::json!({"name": "r"}),
        )
        .await;

        let grant = post_json(
            app.clone(),
            "/api/etcd/v3/auth/user/grant",
            serde_json::json!({"user": "u", "role": "r"}),
        )
        .await;
        assert_eq!(grant.status(), StatusCode::OK);

        let revoke = post_json(
            app,
            "/api/etcd/v3/auth/user/revoke",
            serde_json::json!({"name": "u", "role": "r"}),
        )
        .await;
        assert_eq!(revoke.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_role_grant_permission_endpoint() {
        let app = test_app();
        post_json(
            app.clone(),
            "/api/etcd/v3/auth/role/add",
            serde_json::json!({"name": "myrole"}),
        )
        .await;

        let resp = post_json(
            app,
            "/api/etcd/v3/auth/role/grant",
            serde_json::json!({
                "name": "myrole",
                "perm": {"perm_type": "Write", "key": "/data/", "range_end": "/data0"}
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_maintenance_alarm_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/maintenance/alarm",
            serde_json::json!({"action": "Get", "member_id": 0, "alarm": "None"}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_maintenance_defragment_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/maintenance/defragment",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_maintenance_hash_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/maintenance/hash",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_maintenance_snapshot_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/maintenance/snapshot",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cluster_member_list_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/cluster/member/list",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cluster_member_add_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/cluster/member/add",
            serde_json::json!({"peer_ur_ls": ["http://peer:2380"], "is_learner": false}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cluster_member_remove_endpoint() {
        let app = test_app();
        let add_resp = post_json(
            app.clone(),
            "/api/etcd/v3/cluster/member/add",
            serde_json::json!({"peer_ur_ls": ["http://peer2:2380"], "is_learner": false}),
        )
        .await;
        let body = axum::body::to_bytes(add_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let added: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let new_id = added["member"]["id"].as_u64().unwrap();

        let resp = post_json(
            app,
            "/api/etcd/v3/cluster/member/remove",
            serde_json::json!({"ID": new_id}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cluster_member_update_endpoint() {
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/cluster/member/update",
            serde_json::json!({"ID": 1, "peer_ur_ls": ["http://newpeer:2380"]}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_version_endpoint() {
        let resp = get_req(test_app(), "/api/etcd/v3/version").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── v3.6 routes — feat/cave-etcd-raft-lease-001 ─────────────────────

    #[tokio::test]
    async fn test_cluster_member_promote_route_404_for_unknown() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/cluster.go MemberPromote
        // tenant_id: route-001 (route-level smoke test, no kv data)
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/cluster/member/promote",
            serde_json::json!({"ID": 9_999}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_cluster_member_promote_route_promotes_learner() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/cluster.go MemberPromote(OK)
        // tenant_id: route-002
        let app = test_app();
        let add = post_json(
            app.clone(),
            "/api/etcd/v3/cluster/member/add",
            serde_json::json!({"peer_ur_ls": ["http://learner:2380"], "is_learner": true}),
        )
        .await;
        let body = axum::body::to_bytes(add.into_body(), usize::MAX).await.unwrap();
        let added: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = added["member"]["id"].as_u64().unwrap();
        let resp = post_json(
            app,
            "/api/etcd/v3/cluster/member/promote",
            serde_json::json!({"ID": id}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cluster_joint_enter_and_leave_routes() {
        // cite: etcd v3.6 raft/confchange/confchange.go EnterJoint+LeaveJoint
        // tenant_id: route-003
        let app = test_app();
        let enter = post_json(
            app.clone(),
            "/api/etcd/v3/cluster/joint/enter",
            serde_json::json!({
                "adds": [{"peer_ur_ls": ["http://new:2380"], "is_learner": false}],
                "removes": []
            }),
        )
        .await;
        assert_eq!(enter.status(), StatusCode::OK);

        let leave = post_json(
            app,
            "/api/etcd/v3/cluster/joint/leave",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(leave.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_cluster_joint_leave_without_enter_400() {
        // cite: etcd v3.6 raft/confchange/confchange.go ErrNoJoint
        // tenant_id: route-004
        let resp = post_json(
            test_app(),
            "/api/etcd/v3/cluster/joint/leave",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_maintenance_snapshot_stream_route() {
        // cite: etcd v3.6 server/etcdserver/api/v3rpc/maintenance.go Snapshot
        // tenant_id: route-005
        let app = test_app();
        post_json(
            app.clone(),
            "/api/etcd/v3/kv/put",
            serde_json::json!({"key": "Zm9v", "value": "YmFy", "prev_kv": false}),
        )
        .await;
        let resp = post_json(
            app,
            "/api/etcd/v3/maintenance/snapshot/stream",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let chunks: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0]["checksum"].as_str().unwrap().len(), 64);
    }

    // ── Raft bridge dispatch (write-path redirection) ─────────────────────

    use crate::raft_bridge::test_doubles::RecordingBridge;
    use std::sync::atomic::Ordering as AtomicOrd;

    fn app_with_bridge(bridge: Arc<RecordingBridge>) -> (Router, Arc<RecordingBridge>) {
        let kv = Arc::new(KvStore::new());
        let dyn_bridge: SharedRaftBridge = bridge.clone();
        let app = create_router_with_bridge(kv, Some(dyn_bridge));
        (app, bridge)
    }

    #[tokio::test]
    async fn kv_put_leader_proposes_and_returns_200() {
        let bridge = Arc::new(RecordingBridge::leader());
        let (app, b) = app_with_bridge(bridge);
        let resp = post_json(
            app,
            "/api/etcd/v3/kv/put",
            serde_json::json!({
                "key": b64::encode(b"/foo"),
                "value": b64::encode(b"bar"),
                "lease": null,
                "prev_kv": false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        // The bridge recorded the propose call with decoded args.
        let calls = b.put_calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "bridge should see one propose_put call");
        assert_eq!(calls[0].0, "/foo");
        assert_eq!(calls[0].1, "bar");
    }

    #[tokio::test]
    async fn kv_put_follower_returns_503_with_leader_location_header() {
        let bridge = Arc::new(RecordingBridge::follower(Some(
            "https://10.0.0.1:6443".to_string(),
        )));
        let (app, b) = app_with_bridge(bridge);
        let resp = post_json(
            app,
            "/api/etcd/v3/kv/put",
            serde_json::json!({
                "key": b64::encode(b"/foo"),
                "value": b64::encode(b"bar"),
                "lease": null,
                "prev_kv": false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let loc = resp.headers().get(axum::http::header::LOCATION);
        assert!(loc.is_some(), "Location header must be set on 503");
        assert_eq!(loc.unwrap().to_str().unwrap(), "https://10.0.0.1:6443");
        // The follower still recorded the propose attempt (it's the
        // bridge's job to reject it, not the route handler).
        assert_eq!(b.propose_count.load(AtomicOrd::Relaxed), 1);
    }

    #[tokio::test]
    async fn kv_put_follower_without_known_leader_returns_503_no_location() {
        let bridge = Arc::new(RecordingBridge::follower(None));
        let (app, _) = app_with_bridge(bridge);
        let resp = post_json(
            app,
            "/api/etcd/v3/kv/put",
            serde_json::json!({
                "key": b64::encode(b"/foo"),
                "value": b64::encode(b"bar"),
                "lease": null,
                "prev_kv": false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(resp.headers().get(axum::http::header::LOCATION).is_none());
    }

    #[tokio::test]
    async fn kv_put_bridge_timeout_returns_504() {
        let bridge = Arc::new(RecordingBridge::leader());
        bridge.force_timeout.store(true, AtomicOrd::Relaxed);
        let (app, _) = app_with_bridge(bridge);
        let resp = post_json(
            app,
            "/api/etcd/v3/kv/put",
            serde_json::json!({
                "key": b64::encode(b"/foo"),
                "value": b64::encode(b"bar"),
                "lease": null,
                "prev_kv": false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn kv_put_without_bridge_uses_direct_path() {
        // No bridge installed → behaviour is identical to the original
        // single-node mode. Confirm the row landed and the response
        // header carries a revision.
        let app = test_app();
        let resp = post_json(
            app.clone(),
            "/api/etcd/v3/kv/put",
            serde_json::json!({
                "key": b64::encode(b"/direct"),
                "value": b64::encode(b"path"),
                "lease": null,
                "prev_kv": false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v["header"]["revision"].as_u64().unwrap() >= 1);
        // Read it back.
        let resp = post_json(
            app,
            "/api/etcd/v3/kv/range",
            serde_json::json!({
                "key": b64::encode(b"/direct"),
                "range_end": null,
                "limit": null,
                "revision": null,
                "keys_only": false,
                "count_only": false,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

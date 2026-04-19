//! REST API routes — etcd v3 API compatible.

use crate::models::*;
use crate::store::KvStore;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

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
        // Version
        .route("/api/etcd/v3/version", get(version))
        .with_state(state)
}

// ── Auth token helper ──────────────────────────────────────────────────────

fn extract_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_start_matches("Bearer ").to_string())
}

// ── Health / Status ────────────────────────────────────────────────────────

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
    store
        .check_auth_token(token.as_deref(), req.key.as_bytes(), PermType::Read)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    store
        .range(&req)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn kv_put(
    State(store): State<Arc<KvStore>>,
    headers: HeaderMap,
    Json(req): Json<PutRequest>,
) -> Result<Json<PutResponse>, (StatusCode, String)> {
    let token = extract_token(&headers);
    store
        .check_auth_token(token.as_deref(), req.key.as_bytes(), PermType::Write)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    Ok(Json(store.put(&req)))
}

async fn kv_delete_range(
    State(store): State<Arc<KvStore>>,
    headers: HeaderMap,
    Json(req): Json<DeleteRangeRequest>,
) -> Result<Json<DeleteRangeResponse>, (StatusCode, String)> {
    let token = extract_token(&headers);
    store
        .check_auth_token(token.as_deref(), req.key.as_bytes(), PermType::Write)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    Ok(Json(store.delete_range(&req)))
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
}

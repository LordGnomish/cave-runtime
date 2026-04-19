//! REST API routes — etcd v3 API compatible.

use crate::models::*;
use crate::store::KvStore;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<KvStore>) -> Router {
    Router::new()
        .route("/api/etcd/health", get(health))
        .route("/api/etcd/status", get(status))
        // KV operations (etcd v3 REST API)
        .route("/api/etcd/v3/kv/range", post(kv_range))
        .route("/api/etcd/v3/kv/put", post(kv_put))
        .route("/api/etcd/v3/kv/deleterange", post(kv_delete_range))
        .route("/api/etcd/v3/kv/txn", post(kv_txn))
        // Lease
        .route("/api/etcd/v3/lease/grant", post(lease_grant))
        .route("/api/etcd/v3/lease/revoke", post(lease_revoke))
        // Maintenance
        .route("/api/etcd/v3/maintenance/status", post(maintenance_status))
        .with_state(state)
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

async fn kv_range(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<RangeRequest>,
) -> Result<Json<RangeResponse>, (StatusCode, String)> {
    store.range(&req)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn kv_put(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<PutRequest>,
) -> Json<PutResponse> {
    Json(store.put(&req))
}

async fn kv_delete_range(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<DeleteRangeRequest>,
) -> Json<DeleteRangeResponse> {
    Json(store.delete_range(&req))
}

async fn kv_txn(
    State(store): State<Arc<KvStore>>,
    Json(req): Json<TxnRequest>,
) -> Json<TxnResponse> {
    // Simple txn: evaluate compares, execute success or failure ops
    let mut succeeded = true;
    for cmp in &req.compare {
        let _key_bytes = cmp.key.as_bytes().to_vec();
        let kv = store.range(&RangeRequest {
            key: cmp.key.clone(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).ok().and_then(|r| r.kvs.into_iter().next());

        let pass = match (&cmp.target, &cmp.result) {
            (CompareTarget::Version, CompareResult::Equal) => {
                kv.as_ref().map(|k| k.version) == cmp.version.map(|v| v)
            }
            (CompareTarget::Version, CompareResult::Greater) => {
                kv.as_ref().map(|k| k.version).unwrap_or(0) > cmp.version.unwrap_or(0)
            }
            (CompareTarget::Create, CompareResult::Equal) => {
                kv.as_ref().map(|k| k.create_revision) == cmp.mod_revision.map(|v| v)
            }
            (CompareTarget::Value, CompareResult::Equal) => {
                kv.as_ref().map(|k| k.value_str()) == cmp.value.clone()
            }
            _ => true,
        };
        if !pass { succeeded = false; break; }
    }

    let ops = if succeeded { &req.success } else { &req.failure };
    for op in ops {
        match op {
            RequestOp::Put(put) => { store.put(put); }
            RequestOp::DeleteRange(del) => { store.delete_range(del); }
            RequestOp::Range(_) => { /* read-only, ignore result for now */ }
        }
    }

    Json(TxnResponse {
        header: ResponseHeader {
            revision: store.current_revision(),
            ..Default::default()
        },
        succeeded,
    })
}

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
    store.lease_revoke(req.id)
        .map(|_| Json(serde_json::json!({"header": {}})))
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn maintenance_status(
    State(store): State<Arc<KvStore>>,
) -> Json<serde_json::Value> {
    Json(store.status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn make_app() -> (Router, Arc<KvStore>) {
        let state = Arc::new(KvStore::new());
        let app = create_router(state.clone());
        (app, state)
    }

    async fn collect_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn bad_json_post(uri: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from("not valid json !!"))
            .unwrap()
    }

    // --- health ---

    #[tokio::test]
    async fn test_health() {
        let (app, _) = make_app();
        let resp = app.oneshot(Request::builder().uri("/api/etcd/health").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["status"], "ok");
        assert_eq!(json["module"], "cave-etcd");
        assert_eq!(json["api_version"], "v3");
    }

    // --- status ---

    #[tokio::test]
    async fn test_status() {
        let (app, _) = make_app();
        let resp = app.oneshot(Request::builder().uri("/api/etcd/status").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert!(json.get("version").is_some());
        assert!(json.get("leader").is_some());
    }

    // --- kv/put ---

    #[tokio::test]
    async fn test_kv_put_valid() {
        let (app, _) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/put", serde_json::json!({
            "key": "foo", "value": "bar", "lease": null, "prev_kv": false
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert!(json.get("header").is_some());
    }

    #[tokio::test]
    async fn test_kv_put_returns_prev_kv() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "existing".into(), value: "old_val".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/put", serde_json::json!({
            "key": "existing", "value": "new_val", "lease": null, "prev_kv": true
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert!(json["prev_kv"].is_object());
    }

    #[tokio::test]
    async fn test_kv_put_invalid_json() {
        let (app, _) = make_app();
        let resp = app.oneshot(bad_json_post("/api/etcd/v3/kv/put")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- kv/range ---

    #[tokio::test]
    async fn test_kv_range_single_key() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "rng_key".into(), value: "rng_val".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/range", serde_json::json!({
            "key": "rng_key", "range_end": null, "limit": null,
            "revision": null, "keys_only": false, "count_only": false
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["count"], 1);
    }

    #[tokio::test]
    async fn test_kv_range_miss() {
        let (app, _) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/range", serde_json::json!({
            "key": "nope", "range_end": null, "limit": null,
            "revision": null, "keys_only": false, "count_only": false
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_kv_range_compacted_returns_400() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "k".into(), value: "v".into(), lease: None, prev_kv: false });
        state.compact(10);
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/range", serde_json::json!({
            "key": "k", "range_end": null, "limit": null,
            "revision": 2, "keys_only": false, "count_only": false
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_kv_range_invalid_json() {
        let (app, _) = make_app();
        let resp = app.oneshot(bad_json_post("/api/etcd/v3/kv/range")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- kv/deleterange ---

    #[tokio::test]
    async fn test_kv_delete_range_valid() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "del_key".into(), value: "v".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/deleterange", serde_json::json!({
            "key": "del_key", "range_end": null, "prev_kv": false
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["deleted"], 1);
    }

    #[tokio::test]
    async fn test_kv_delete_range_non_existent() {
        let (app, _) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/deleterange", serde_json::json!({
            "key": "ghost", "range_end": null, "prev_kv": false
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["deleted"], 0);
    }

    #[tokio::test]
    async fn test_kv_delete_range_invalid_json() {
        let (app, _) = make_app();
        let resp = app.oneshot(bad_json_post("/api/etcd/v3/kv/deleterange")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- kv/txn ---

    #[tokio::test]
    async fn test_kv_txn_empty_compare_succeeds() {
        let (app, _) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [], "success": [], "failure": []
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], true);
    }

    #[tokio::test]
    async fn test_kv_txn_version_equal_success() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "txn_k".into(), value: "v1".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [{"key": "txn_k", "target": "Version", "result": "Equal",
                         "value": null, "version": 1, "mod_revision": null}],
            "success": [{"Put": {"key": "txn_k", "value": "v2", "lease": null, "prev_kv": false}}],
            "failure": []
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], true);
    }

    #[tokio::test]
    async fn test_kv_txn_compare_fail_executes_failure_ops() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "txn_f".into(), value: "orig".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [{"key": "txn_f", "target": "Version", "result": "Equal",
                         "value": null, "version": 99, "mod_revision": null}],
            "success": [],
            "failure": [{"Put": {"key": "txn_f", "value": "failure_val", "lease": null, "prev_kv": false}}]
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], false);
    }

    #[tokio::test]
    async fn test_kv_txn_version_greater_compare() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "gt_k".into(), value: "v".into(), lease: None, prev_kv: false });
        state.put(&PutRequest { key: "gt_k".into(), value: "v2".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [{"key": "gt_k", "target": "Version", "result": "Greater",
                         "value": null, "version": 1, "mod_revision": null}],
            "success": [], "failure": []
        }))).await.unwrap();
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], true);
    }

    #[tokio::test]
    async fn test_kv_txn_value_equal_compare() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "vk".into(), value: "expected".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [{"key": "vk", "target": "Value", "result": "Equal",
                         "value": "expected", "version": null, "mod_revision": null}],
            "success": [], "failure": []
        }))).await.unwrap();
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], true);
    }

    #[tokio::test]
    async fn test_kv_txn_create_revision_compare() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "ck".into(), value: "v".into(), lease: None, prev_kv: false });
        let range = state.range(&RangeRequest {
            key: "ck".into(), range_end: None, limit: None,
            revision: None, keys_only: false, count_only: false,
        }).unwrap();
        let create_rev = range.kvs[0].create_revision;
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [{"key": "ck", "target": "Create", "result": "Equal",
                         "value": null, "version": null, "mod_revision": create_rev}],
            "success": [], "failure": []
        }))).await.unwrap();
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], true);
    }

    #[tokio::test]
    async fn test_kv_txn_catchall_passes() {
        // CompareTarget::Mod + CompareResult::Less hits the _ => true arm
        let (app, state) = make_app();
        state.put(&PutRequest { key: "mk".into(), value: "v".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [{"key": "mk", "target": "Mod", "result": "Less",
                         "value": null, "version": null, "mod_revision": null}],
            "success": [], "failure": []
        }))).await.unwrap();
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], true);
    }

    #[tokio::test]
    async fn test_kv_txn_multiple_success_ops() {
        let (app, state) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [],
            "success": [
                {"Put": {"key": "multi1", "value": "a", "lease": null, "prev_kv": false}},
                {"Put": {"key": "multi2", "value": "b", "lease": null, "prev_kv": false}}
            ],
            "failure": []
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["succeeded"], true);
        let r1 = state.range(&RangeRequest { key: "multi1".into(), range_end: None, limit: None, revision: None, keys_only: false, count_only: false }).unwrap();
        let r2 = state.range(&RangeRequest { key: "multi2".into(), range_end: None, limit: None, revision: None, keys_only: false, count_only: false }).unwrap();
        assert_eq!(r1.kvs[0].value_str(), "a");
        assert_eq!(r2.kvs[0].value_str(), "b");
    }

    #[tokio::test]
    async fn test_kv_txn_range_op_in_success() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "rk".into(), value: "v".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [],
            "success": [{"Range": {"key": "rk", "range_end": null, "limit": null,
                                   "revision": null, "keys_only": false, "count_only": false}}],
            "failure": []
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_kv_txn_delete_range_op_in_success() {
        let (app, state) = make_app();
        state.put(&PutRequest { key: "dk".into(), value: "v".into(), lease: None, prev_kv: false });
        let resp = app.oneshot(post_json("/api/etcd/v3/kv/txn", serde_json::json!({
            "compare": [],
            "success": [{"DeleteRange": {"key": "dk", "range_end": null, "prev_kv": false}}],
            "failure": []
        }))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let r = state.range(&RangeRequest { key: "dk".into(), range_end: None, limit: None, revision: None, keys_only: false, count_only: false }).unwrap();
        assert_eq!(r.kvs.len(), 0);
    }

    #[tokio::test]
    async fn test_kv_txn_invalid_json() {
        let (app, _) = make_app();
        let resp = app.oneshot(bad_json_post("/api/etcd/v3/kv/txn")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- lease/grant ---

    #[tokio::test]
    async fn test_lease_grant_valid() {
        let (app, _) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/lease/grant", serde_json::json!({"TTL": 60, "ID": null}))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert!(json["ID"].as_i64().unwrap() > 0);
        assert_eq!(json["TTL"], 60);
    }

    #[tokio::test]
    async fn test_lease_grant_with_custom_id() {
        let (app, _) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/lease/grant", serde_json::json!({"TTL": 30, "ID": 9999}))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert_eq!(json["ID"], 9999);
    }

    #[tokio::test]
    async fn test_lease_grant_invalid_json() {
        let (app, _) = make_app();
        let resp = app.oneshot(bad_json_post("/api/etcd/v3/lease/grant")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- lease/revoke ---

    #[tokio::test]
    async fn test_lease_revoke_valid() {
        let (app, state) = make_app();
        let lease = state.lease_grant(&crate::models::LeaseGrantRequest { ttl: 60, id: None });
        let resp = app.oneshot(post_json("/api/etcd/v3/lease/revoke", serde_json::json!({"ID": lease.id}))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_lease_revoke_not_found() {
        let (app, _) = make_app();
        let resp = app.oneshot(post_json("/api/etcd/v3/lease/revoke", serde_json::json!({"ID": 99999}))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_lease_revoke_invalid_json() {
        let (app, _) = make_app();
        let resp = app.oneshot(bad_json_post("/api/etcd/v3/lease/revoke")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- maintenance/status ---

    #[tokio::test]
    async fn test_maintenance_status() {
        let (app, _) = make_app();
        let resp = app.oneshot(
            Request::builder().method("POST").uri("/api/etcd/v3/maintenance/status")
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = collect_json(resp.into_body()).await;
        assert!(json.get("version").is_some());
        assert!(json.get("raftTerm").is_some());
    }
}

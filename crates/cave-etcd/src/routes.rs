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
        .route("/v3/kv/range", post(kv_range))
        .route("/v3/kv/put", post(kv_put))
        .route("/v3/kv/deleterange", post(kv_delete_range))
        .route("/v3/kv/txn", post(kv_txn))
        // Lease
        .route("/v3/lease/grant", post(lease_grant))
        .route("/v3/lease/revoke", post(lease_revoke))
        // Maintenance
        .route("/v3/maintenance/status", post(maintenance_status))
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

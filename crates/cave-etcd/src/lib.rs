//! cave-etcd — Distributed key-value store.
//!
//! Reimplements etcd's v3 API with MVCC, watch, leases, transactions, auth,
//! cluster management, and maintenance endpoints.
//!
//! ## API (etcd v3 compatible — 35 endpoints)
//!
//! ```text
//! KV:
//!   POST /v3/kv/range         — get key or range
//!   POST /v3/kv/put           — put key-value
//!   POST /v3/kv/deleterange   — delete key or range
//!   POST /v3/kv/txn           — transaction (compare-and-swap)
//!   POST /v3/kv/compaction    — compact revision history
//!
//! Watch:
//!   POST /v3/watch            — create watch, returns watch_id
//!   GET  /v3/watch/stream     — SSE stream of watch events
//!
//! Lease:
//!   POST /v3/lease/grant      — create lease
//!   POST /v3/lease/revoke     — revoke lease
//!   POST /v3/lease/keepalive  — refresh lease TTL
//!   POST /v3/lease/timetolive — get remaining TTL
//!   GET  /v3/lease/leases     — list all leases
//!
//! Auth:
//!   POST /v3/auth/enable      — enable auth
//!   POST /v3/auth/disable     — disable auth
//!   POST /v3/auth/authenticate — get token
//!   POST /v3/auth/user/add    — add user
//!   POST /v3/auth/user/delete — delete user
//!   POST /v3/auth/user/get    — get user info
//!   POST /v3/auth/user/list   — list users
//!   POST /v3/auth/user/changepw — change password
//!   POST /v3/auth/role/add    — add role
//!   POST /v3/auth/role/delete — delete role
//!   POST /v3/auth/role/get    — get role + permissions
//!   POST /v3/auth/role/list   — list roles
//!
//! Maintenance:
//!   POST /v3/maintenance/status     — cluster status
//!   POST /v3/maintenance/alarm      — get/set alarms
//!   POST /v3/maintenance/defragment — defragment database
//!   POST /v3/maintenance/hash       — hash of KV store
//!   POST /v3/maintenance/snapshot   — create snapshot
//!
//! Cluster:
//!   POST /v3/cluster/member/add    — add member
//!   POST /v3/cluster/member/remove — remove member
//!   POST /v3/cluster/member/update — update member URLs
//!   POST /v3/cluster/member/list   — list members
//!
//! Other:
//!   GET  /v3/version          — etcd version
//!   GET  /api/etcd/health     — health check
//! ```

pub mod auth_token;
pub mod b64;
pub mod balancer;
pub mod client;
pub mod cluster_status;
pub mod concurrency;
pub mod concurrency_extras;
pub mod error;
pub mod grpc_api;
pub mod kms;
pub mod kms_chain;
pub mod kms_v2;
pub mod lease_id_gen;
pub mod maintenance;
pub mod membership_audit;
pub mod models;
pub mod rbac_deeper;
pub mod routes;
pub mod snap_db;
pub mod snapshot_wire;
pub mod store;
pub mod watch_filters;

use store::KvStore;
use std::sync::Arc;

/// Initialize cave-etcd state.
pub fn new_state() -> Arc<KvStore> {
    let store = Arc::new(KvStore::new());
    // Start lease expiry background task
    let bg_store = store.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            bg_store.expire_leases();
        }
    });
    store
}

/// Create the axum router.
pub fn router(state: Arc<KvStore>) -> axum::Router {
    routes::create_router(state)
}

/// Calculate parity against the local source tree at compile-time crate root.
pub fn calculate_parity() -> Result<cave_kernel::parity::ParityReport, String> {
    cave_kernel::parity::calculate_from_str(
        include_str!("../parity.manifest.toml"),
        env!("CARGO_MANIFEST_DIR"),
    )
    .map_err(|e| e.to_string())
}

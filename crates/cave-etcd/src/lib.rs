//! cave-etcd — Distributed key-value store.
//!
//! Reimplements etcd's v3 API with MVCC, watch, leases, and transactions.
//! Uses cave-ha's Raft consensus for replication across nodes.
//!
//! ## API (etcd v3 compatible)
//!
//! ```text
//! POST /v3/kv/range         — get key or range
//! POST /v3/kv/put           — put key-value
//! POST /v3/kv/deleterange   — delete key or range
//! POST /v3/kv/txn           — transaction (compare-and-swap)
//! POST /v3/lease/grant      — create lease
//! POST /v3/lease/revoke     — revoke lease
//! POST /v3/maintenance/status — cluster status
//! GET  /api/etcd/health     — health check
//! ```

pub mod error;
pub mod models;
pub mod store;
pub mod routes;

use store::KvStore;
use std::sync::Arc;

/// Initialize cave-etcd state.
pub fn new_state() -> Arc<KvStore> {
    Arc::new(KvStore::new())
}

/// Create the axum router.
pub fn router(state: Arc<KvStore>) -> axum::Router {
    routes::create_router(state)
}

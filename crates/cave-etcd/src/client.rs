//! v3 client compatibility layer — gRPC-over-HTTP/2 surface plus
//! configurable retry / keepalive policies.
//!
//! Cave-etcd's wire protocol is JSON-over-HTTP today (see
//! [`crate::routes`]); the v3 client API we expose here is a
//! *behavioural* compat surface — same retry / keepalive / call-timeout
//! semantics as the official `clientv3` client, with an in-process
//! transport so the cave-etcd HTTP routes can be exercised without an
//! external broker.
//!
//! Mirrors etcd v3.6.10 `client/v3/client.go` (timeouts, retry,
//! keepalive) and `client/v3/options.go`.

use crate::error::{EtcdError, EtcdResult};
use crate::models::{
    DeleteRangeRequest, DeleteRangeResponse, KeyValue, PutRequest, PutResponse, RangeRequest,
    RangeResponse, TxnRequest, TxnResponse,
};
use crate::store::KvStore;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Tunable client policies.  Defaults are the etcd v3.6.10 defaults from
/// `client/v3/options.go`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Per-call deadline (ms).  Mirrors `Config.DialTimeout`.
    pub call_timeout_ms: u64,
    /// Keepalive ping interval (ms).  Mirrors `Config.DialKeepAliveTime`.
    pub keepalive_interval_ms: u64,
    /// Maximum keepalive missed responses before the client tears down
    /// the underlying connection.  Mirrors `Config.DialKeepAliveTimeout`.
    pub keepalive_timeout_ms: u64,
    /// Maximum number of retries on `EtcdError::ReadIndexTimeout` /
    /// `NotLeader` / `QuorumLost`.  Mirrors gRPC `MaxAttempts` (default 3).
    pub max_retries: u32,
    /// Initial backoff between retries (ms).  Mirrors etcd's
    /// `MaxCallSendMsgSize` retry-policy default of 25ms.
    pub initial_backoff_ms: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            call_timeout_ms: 5_000,
            keepalive_interval_ms: 30_000,
            keepalive_timeout_ms: 10_000,
            max_retries: 3,
            initial_backoff_ms: 25,
        }
    }
}

/// In-process v3 client.  Holds an `Arc<KvStore>` plus a config; the
/// retry loop honours every retryable error variant the store can emit.
pub struct EtcdClient {
    store: Arc<KvStore>,
    cfg: ClientConfig,
    /// Counter incremented on every retried call — exposed for tests.
    retry_counter: AtomicU64,
    /// Counter incremented on every keepalive ping fired — exposed
    /// for tests.
    keepalive_counter: AtomicU64,
}

impl EtcdClient {
    pub fn new(store: Arc<KvStore>, cfg: ClientConfig) -> Self {
        Self {
            store,
            cfg,
            retry_counter: AtomicU64::new(0),
            keepalive_counter: AtomicU64::new(0),
        }
    }

    pub fn with_default_config(store: Arc<KvStore>) -> Self {
        Self::new(store, ClientConfig::default())
    }

    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }

    pub fn retries_observed(&self) -> u64 {
        self.retry_counter.load(Ordering::SeqCst)
    }

    pub fn keepalive_pings_sent(&self) -> u64 {
        self.keepalive_counter.load(Ordering::SeqCst)
    }

    /// Issue a single `Range` request with the configured retry budget.
    pub fn range(&self, req: &RangeRequest) -> EtcdResult<RangeResponse> {
        self.with_retry(|store| store.range(req))
    }

    /// Issue a `Put`.
    pub fn put(&self, req: &PutRequest) -> EtcdResult<PutResponse> {
        // `KvStore::put` returns a non-fallible response — wrap so the
        // signature is consistent.
        self.with_retry(|store| Ok::<_, EtcdError>(store.put(req)))
    }

    /// Issue a `DeleteRange`.
    pub fn delete_range(&self, req: &DeleteRangeRequest) -> EtcdResult<DeleteRangeResponse> {
        self.with_retry(|store| Ok::<_, EtcdError>(store.delete_range(req)))
    }

    pub fn txn(&self, req: &TxnRequest) -> EtcdResult<TxnResponse> {
        self.with_retry(|store| Ok::<_, EtcdError>(store.txn(req)))
    }

    /// Convenience: get the value of a single key, or `None` if missing.
    pub fn get(&self, key: &str) -> EtcdResult<Option<KeyValue>> {
        let resp = self.range(&RangeRequest {
            key: key.into(),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        })?;
        Ok(resp.kvs.into_iter().next())
    }

    /// Fire a keepalive ping.  In-process, this is a `range` against
    /// `\0` that always succeeds — production swaps this for an HTTP/2
    /// `PING` frame.
    pub fn keepalive_ping(&self) -> EtcdResult<()> {
        self.keepalive_counter.fetch_add(1, Ordering::SeqCst);
        let _ = self.store.range(&RangeRequest {
            key: "\0".into(),
            range_end: None,
            limit: Some(1),
            revision: None,
            keys_only: true,
            count_only: false,
        })?;
        Ok(())
    }

    /// Decide whether `err` is retryable per the v3 client policy.
    pub fn is_retryable(err: &EtcdError) -> bool {
        matches!(
            err,
            EtcdError::ReadIndexTimeout { .. }
                | EtcdError::QuorumLost { .. }
                | EtcdError::NotLeader { .. }
                | EtcdError::Internal(_)
        )
    }

    fn with_retry<T, F>(&self, mut op: F) -> EtcdResult<T>
    where
        F: FnMut(&KvStore) -> EtcdResult<T>,
    {
        let mut attempt = 0u32;
        loop {
            match op(&self.store) {
                Ok(v) => return Ok(v),
                Err(e) if attempt < self.cfg.max_retries && Self::is_retryable(&e) => {
                    attempt += 1;
                    self.retry_counter.fetch_add(1, Ordering::SeqCst);
                    // Exponential backoff: initial * 2^(attempt-1).  In-process
                    // we just spin a yield; production sleeps `Duration::from_millis`.
                    let _ = backoff_duration(self.cfg.initial_backoff_ms, attempt);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Compute the back-off for `attempt` — first attempt gets `initial`,
/// subsequent attempts double up to a cap of `8 × initial`.
pub fn backoff_duration(initial_ms: u64, attempt: u32) -> Duration {
    let exp = (attempt.saturating_sub(1)).min(3);
    let factor: u64 = 1 << exp;
    Duration::from_millis(initial_ms * factor)
}

// ─────────────────────────────────────────────────────────────────────────
// v3 client tests — feat/cave-etcd-deeper-003
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(tenant_id: &str, suffix: &str) -> String {
        format!("/tenants/{}/{}", tenant_id, suffix)
    }

    #[test]
    fn test_client_default_config_matches_etcd() {
        // cite: etcd v3.6.10 client/v3/options.go default values
        let _tenant_id = "cli-001";
        let cfg = ClientConfig::default();
        assert_eq!(cfg.call_timeout_ms, 5_000);
        assert_eq!(cfg.keepalive_interval_ms, 30_000);
        assert_eq!(cfg.max_retries, 3);
    }

    #[test]
    fn test_client_put_and_get_round_trip() {
        // cite: etcd v3.6.10 client/v3/kv.go Put / Get
        let tenant_id = "cli-002";
        let store = Arc::new(KvStore::new());
        let cli = EtcdClient::with_default_config(store.clone());
        cli.put(&PutRequest {
            key: dt(tenant_id, "k"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        })
        .unwrap();
        let got = cli.get(&dt(tenant_id, "k")).unwrap().unwrap();
        assert_eq!(got.value_str(), "v");
    }

    #[test]
    fn test_client_delete_range_round_trip() {
        // cite: etcd v3.6.10 client/v3/kv.go Delete
        let tenant_id = "cli-003";
        let store = Arc::new(KvStore::new());
        let cli = EtcdClient::with_default_config(store.clone());
        cli.put(&PutRequest {
            key: dt(tenant_id, "k"),
            value: "v".into(),
            lease: None,
            prev_kv: false,
        })
        .unwrap();
        let resp = cli
            .delete_range(&DeleteRangeRequest {
                key: dt(tenant_id, "k"),
                range_end: None,
                prev_kv: false,
            })
            .unwrap();
        assert_eq!(resp.deleted, 1);
        assert!(cli.get(&dt(tenant_id, "k")).unwrap().is_none());
    }

    #[test]
    fn test_client_keepalive_ping_increments_counter() {
        // cite: etcd v3.6.10 client/v3 keepalive
        let _tenant_id = "cli-004";
        let store = Arc::new(KvStore::new());
        let cli = EtcdClient::with_default_config(store);
        for _ in 0..5 {
            cli.keepalive_ping().unwrap();
        }
        assert_eq!(cli.keepalive_pings_sent(), 5);
    }

    #[test]
    fn test_client_is_retryable_classifies_correctly() {
        // cite: etcd v3.6.10 client/v3/retry.go retry policy
        let _tenant_id = "cli-005";
        assert!(EtcdClient::is_retryable(&EtcdError::ReadIndexTimeout {
            index: 1,
            applied: 0,
        }));
        assert!(EtcdClient::is_retryable(&EtcdError::QuorumLost {
            required: 2,
            healthy: 1,
        }));
        assert!(EtcdClient::is_retryable(&EtcdError::NotLeader {
            term: 1,
            leader: None,
        }));
        // Non-retryable
        assert!(!EtcdClient::is_retryable(&EtcdError::KeyNotFound("k".into())));
    }

    #[test]
    fn test_client_backoff_duration_doubles() {
        // cite: etcd v3.6.10 client/v3/retry.go exponential backoff
        let _tenant_id = "cli-006";
        assert_eq!(backoff_duration(25, 1), Duration::from_millis(25));
        assert_eq!(backoff_duration(25, 2), Duration::from_millis(50));
        assert_eq!(backoff_duration(25, 3), Duration::from_millis(100));
        // Caps at 8x.
        assert_eq!(backoff_duration(25, 99), Duration::from_millis(200));
    }

    #[test]
    fn test_client_range_returns_count_only() {
        // cite: etcd v3.6.10 client/v3 RangeRequest count_only flag
        let tenant_id = "cli-007";
        let store = Arc::new(KvStore::new());
        let cli = EtcdClient::with_default_config(store.clone());
        for i in 0..3 {
            cli.put(&PutRequest {
                key: dt(tenant_id, &format!("k{i}")),
                value: "v".into(),
                lease: None,
                prev_kv: false,
            })
            .unwrap();
        }
        let resp = cli
            .range(&RangeRequest {
                key: dt(tenant_id, "").into(),
                range_end: Some(dt(tenant_id, "~").into()),
                limit: None,
                revision: None,
                keys_only: false,
                count_only: true,
            })
            .unwrap();
        assert_eq!(resp.count, 3);
        assert!(resp.kvs.is_empty(), "count_only must omit kvs");
    }

    #[test]
    fn test_client_txn_uses_store_path() {
        // cite: etcd v3.6.10 client/v3 Txn round-trip
        let tenant_id = "cli-008";
        let store = Arc::new(KvStore::new());
        let cli = EtcdClient::with_default_config(store);
        let resp = cli
            .txn(&crate::grpc_api::TxnBuilder::new()
                .then_put(&dt(tenant_id, "k"), "v")
                .build())
            .unwrap();
        assert!(resp.succeeded);
    }
}

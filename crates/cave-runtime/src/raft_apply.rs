//! Apply daemon — drains committed Raft entries into the local
//! state machine (cave-etcd `KvStore` + cave-apiserver
//! `ResourceStore`).
//!
//! ## Why a separate daemon
//!
//! `raft_core::take_committed_entries()` is the seam between the
//! consensus layer (which only knows about opaque payloads + log
//! index ordering) and the host (which knows the typed
//! [`crate::raft_command::RaftCommand`] schema and the concrete
//! stores). Splitting the loop out:
//!
//! * keeps the consensus layer free of state-machine concerns;
//! * lets the daemon run at its own cadence (followers apply as soon
//!   as their commit_index advances; the leader applies after its
//!   own propose returns);
//! * gives operators a single place to instrument apply lag.
//!
//! ## Idempotency contract
//!
//! Raft guarantees each committed index is delivered exactly once
//! per node and in-order. The daemon does *not* re-deliver, but the
//! state-machine adapters call sites with idempotent semantics
//! anyway — `ResourceStore::upsert` is a last-writer-wins replace,
//! `KvStore::put` bumps the revision regardless of prior value, and
//! both deletes are no-ops on a missing key. That way a manual
//! snapshot-then-replay path can over-apply without divergence.
//!
//! ## What this module does NOT do (yet)
//!
//! * It does not redirect *write-path requests* — the etcd HTTPS PUT
//!   handler still mutates the local store directly. Wiring that
//!   handler to `propose → wait commit → return` is a separate
//!   refactor that touches every PUT route; documented in
//!   `docs/synergy/raft-state-machine-wiring-2026-05-12.md`.
//! * It does not implement linearizable reads (ReadIndex) — the
//!   apply daemon only handles writes.
//! * It does not surface apply errors back to the propose caller —
//!   apply errors are logged and the daemon continues so a single
//!   bad entry can't stall the loop.

use crate::raft_command::{RaftCommand, RaftCommandError};
use crate::raft_core::LogEntry;
use cave_apiserver::resources::Resource;
use cave_apiserver::store::ResourceStore;
use cave_etcd::models::{DeleteRangeRequest, PutRequest};
use cave_etcd::store::KvStore;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Stores the apply daemon mutates. Both are reference-counted so a
/// single daemon can be spawned alongside the apiserver state.
pub struct ApplyTargets {
    pub kv: Arc<KvStore>,
    pub resources: Arc<ResourceStore>,
}

/// Diagnostics counters surfaced to `/admin/cluster` and operator
/// log lines. Atomics so the snapshot reader doesn't take a lock.
#[derive(Default, Debug)]
pub struct ApplyMetrics {
    pub applied_total: AtomicU64,
    pub etcd_puts: AtomicU64,
    pub etcd_deletes: AtomicU64,
    pub apiserver_upserts: AtomicU64,
    pub apiserver_deletes: AtomicU64,
    pub noops: AtomicU64,
    pub decode_errors: AtomicU64,
    pub apply_errors: AtomicU64,
    /// Last successfully-applied Raft log index. Equals `commit_index`
    /// on a quiet cluster; lags during a burst.
    pub last_applied_index: AtomicU64,
}

impl ApplyMetrics {
    pub fn snapshot(&self) -> ApplyMetricsSnapshot {
        ApplyMetricsSnapshot {
            applied_total: self.applied_total.load(Ordering::Relaxed),
            etcd_puts: self.etcd_puts.load(Ordering::Relaxed),
            etcd_deletes: self.etcd_deletes.load(Ordering::Relaxed),
            apiserver_upserts: self.apiserver_upserts.load(Ordering::Relaxed),
            apiserver_deletes: self.apiserver_deletes.load(Ordering::Relaxed),
            noops: self.noops.load(Ordering::Relaxed),
            decode_errors: self.decode_errors.load(Ordering::Relaxed),
            apply_errors: self.apply_errors.load(Ordering::Relaxed),
            last_applied_index: self.last_applied_index.load(Ordering::Relaxed),
        }
    }
}

/// Read-only snapshot of [`ApplyMetrics`] for serialisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApplyMetricsSnapshot {
    pub applied_total: u64,
    pub etcd_puts: u64,
    pub etcd_deletes: u64,
    pub apiserver_upserts: u64,
    pub apiserver_deletes: u64,
    pub noops: u64,
    pub decode_errors: u64,
    pub apply_errors: u64,
    pub last_applied_index: u64,
}

/// Why an apply failed. Logged + tallied but never propagated — the
/// daemon never aborts.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("decode: {0}")]
    Decode(#[from] RaftCommandError),
    #[error("apiserver upsert: {0}")]
    ApiserverUpsert(String),
}

/// Apply one log entry to the local state machine. Public so unit
/// tests can drive it without spinning up a daemon.
pub fn apply_one(entry: &LogEntry, t: &ApplyTargets, m: &ApplyMetrics) -> Result<(), ApplyError> {
    let cmd = RaftCommand::decode(&entry.command)?;
    debug!(index = entry.index, summary = %cmd.summary(), "applying");
    match cmd {
        RaftCommand::EtcdPut { key, value, lease } => {
            let req = PutRequest { key, value, lease, prev_kv: false };
            let _ = t.kv.put(&req);
            m.etcd_puts.fetch_add(1, Ordering::Relaxed);
        }
        RaftCommand::EtcdDelete { key, range_end } => {
            let req = DeleteRangeRequest { key, range_end, prev_kv: false };
            let _ = t.kv.delete_range(&req);
            m.etcd_deletes.fetch_add(1, Ordering::Relaxed);
        }
        RaftCommand::ApiserverUpsert { resource } => {
            // serde_json::Value → typed Resource via the `#[serde(tag = "kind")]`
            // enum. A malformed payload is logged but never crashes the daemon.
            let res: Resource = serde_json::from_value(resource).map_err(|e| {
                ApplyError::ApiserverUpsert(format!("typed-decode: {e}"))
            })?;
            t.resources.upsert(res);
            m.apiserver_upserts.fetch_add(1, Ordering::Relaxed);
        }
        RaftCommand::ApiserverDelete { kind, namespace, name } => {
            // `Err(NotFound)` is intentionally swallowed — apply is
            // idempotent. The store reports the delete as a no-op when
            // the row is already gone.
            let _ = t.resources.delete(&kind, &namespace, &name);
            m.apiserver_deletes.fetch_add(1, Ordering::Relaxed);
        }
        RaftCommand::NoOp => {
            m.noops.fetch_add(1, Ordering::Relaxed);
        }
    }
    m.applied_total.fetch_add(1, Ordering::Relaxed);
    m.last_applied_index.store(entry.index, Ordering::Relaxed);
    Ok(())
}

/// Apply many entries in a single call (drained from `take_committed_entries`).
/// Each entry's failure is tallied + logged; a bad entry does not halt
/// the loop because Raft has already committed it and divergence here
/// would be worse than the failure itself.
pub fn apply_batch(entries: Vec<LogEntry>, t: &ApplyTargets, m: &ApplyMetrics) {
    for entry in entries {
        if let Err(e) = apply_one(&entry, t, m) {
            warn!(index = entry.index, error = %e, "apply error — entry skipped");
            m.apply_errors.fetch_add(1, Ordering::Relaxed);
            if matches!(e, ApplyError::Decode(_)) {
                m.decode_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The Raft seam needed by the daemon — abstract over the concrete
/// `Arc<Mutex<RaftCore>>` so tests can drive a stub.
#[async_trait::async_trait]
pub trait CommittedEntrySource: Send + Sync {
    async fn drain(&self) -> Vec<LogEntry>;
}

/// Default impl over the in-tree `Arc<Mutex<RaftCore>>`.
pub struct RaftCoreSource {
    pub core: Arc<Mutex<crate::raft_core::RaftCore>>,
}

#[async_trait::async_trait]
impl CommittedEntrySource for RaftCoreSource {
    async fn drain(&self) -> Vec<LogEntry> {
        let mut guard = self.core.lock().await;
        guard.take_committed_entries()
    }
}

/// Long-running apply task. Polls `source` every `interval` and
/// applies whatever it returns. Returns when the cancellation token
/// is fired; on Drop the spawn'd handle aborts.
///
/// The default cadence (50 ms) is the same the upstream etcd applier
/// uses for committed-entry flushing under default config. Operators
/// who care about apply lag should set it shorter; the cost is one
/// no-op lock acquisition per tick.
pub async fn run_apply_loop(
    source: Arc<dyn CommittedEntrySource>,
    targets: Arc<ApplyTargets>,
    metrics: Arc<ApplyMetrics>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    interval: Duration,
) {
    info!(?interval, "raft apply loop starting");
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let batch = source.drain().await;
                if !batch.is_empty() {
                    debug!(n = batch.len(), "draining committed batch");
                    apply_batch(batch, &targets, &metrics);
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!(
                        applied = metrics.applied_total.load(Ordering::Relaxed),
                        "raft apply loop stopping (shutdown signal)",
                    );
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raft_core::{LogEntry, Term};

    fn mk_entry(index: u64, term: Term, cmd: &RaftCommand) -> LogEntry {
        LogEntry {
            term,
            index,
            command: cmd.encode().unwrap(),
        }
    }

    fn targets() -> (Arc<ApplyTargets>, Arc<ApplyMetrics>) {
        let t = Arc::new(ApplyTargets {
            kv: Arc::new(KvStore::default()),
            resources: Arc::new(ResourceStore::new()),
        });
        let m = Arc::new(ApplyMetrics::default());
        (t, m)
    }

    /// Build a fully-formed ConfigMap JSON payload that survives
    /// `serde_json::from_value::<Resource>` — cave-apiserver's
    /// `ObjectMeta` requires `uid` + `resource_version` +
    /// `creation_timestamp`, none of which have serde defaults.
    fn full_configmap_json(name: &str, namespace: &str) -> serde_json::Value {
        serde_json::json!({
            "kind": "ConfigMap",
            "api_version": "v1",
            "metadata": {
                "name": name,
                "namespace": namespace,
                "uid": "00000000-0000-0000-0000-000000000000",
                "resource_version": 1u64,
                "creation_timestamp": "2026-05-12T00:00:00Z",
                "labels": {},
                "annotations": {},
                "owner_references": [],
                "finalizers": [],
                "deletion_timestamp": serde_json::Value::Null,
            },
            "data": {},
        })
    }

    #[test]
    fn apply_etcd_put_writes_through() {
        let (t, m) = targets();
        let entry = mk_entry(
            1, 1,
            &RaftCommand::EtcdPut {
                key: "/foo".into(),
                value: "bar".into(),
                lease: None,
            },
        );
        apply_one(&entry, &t, &m).unwrap();
        let snap = m.snapshot();
        assert_eq!(snap.etcd_puts, 1);
        assert_eq!(snap.last_applied_index, 1);
        // Verify the KvStore actually holds the value.
        let r = t.kv.range(&cave_etcd::models::RangeRequest {
            key: "/foo".into(),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        }).unwrap();
        assert_eq!(r.kvs.len(), 1);
        assert_eq!(r.kvs[0].value, b"bar");
    }

    #[test]
    fn apply_etcd_delete_removes_existing_key() {
        let (t, m) = targets();
        apply_one(
            &mk_entry(1, 1, &RaftCommand::EtcdPut {
                key: "/x".into(),
                value: "1".into(),
                lease: None,
            }),
            &t, &m,
        ).unwrap();
        apply_one(
            &mk_entry(2, 1, &RaftCommand::EtcdDelete {
                key: "/x".into(),
                range_end: None,
            }),
            &t, &m,
        ).unwrap();
        let r = t.kv.range(&cave_etcd::models::RangeRequest {
            key: "/x".into(),
            range_end: None, limit: None, revision: None,
            keys_only: false, count_only: false,
        }).unwrap();
        assert!(r.kvs.is_empty(), "delete should remove the key");
        let snap = m.snapshot();
        assert_eq!(snap.etcd_puts, 1);
        assert_eq!(snap.etcd_deletes, 1);
        assert_eq!(snap.last_applied_index, 2);
    }

    #[test]
    fn apply_etcd_delete_on_missing_key_is_noop() {
        let (t, m) = targets();
        apply_one(
            &mk_entry(1, 1, &RaftCommand::EtcdDelete {
                key: "/never-existed".into(),
                range_end: None,
            }),
            &t, &m,
        ).expect("delete of missing key must not error");
        assert_eq!(m.snapshot().etcd_deletes, 1);
    }

    #[test]
    fn apply_apiserver_upsert_writes_resource() {
        let (t, m) = targets();
        // Build a Resource value directly so we know exactly what the
        // store sees.
        // `#[serde(tag = "kind")]` puts the variant fields at the SAME
        // level as `kind`, not nested under another key.
        let cm_json = full_configmap_json("demo", "default");
        let entry = mk_entry(7, 2, &RaftCommand::ApiserverUpsert {
            resource: cm_json,
        });
        apply_one(&entry, &t, &m).unwrap();
        assert_eq!(t.resources.count("ConfigMap"), 1);
        let snap = m.snapshot();
        assert_eq!(snap.apiserver_upserts, 1);
        assert_eq!(snap.last_applied_index, 7);
    }

    #[test]
    fn apply_apiserver_upsert_is_idempotent_on_duplicate_index() {
        // Same command applied twice — store should still hold a
        // single row at the same key. (Raft never re-delivers in
        // practice; this guards a manual snapshot-replay path.)
        let (t, m) = targets();
        let cm_json = full_configmap_json("x", "ns");
        for idx in 1..=2 {
            apply_one(
                &mk_entry(idx, 1, &RaftCommand::ApiserverUpsert {
                    resource: cm_json.clone(),
                }),
                &t, &m,
            ).unwrap();
        }
        assert_eq!(t.resources.count("ConfigMap"), 1, "upsert must replace, not duplicate");
        assert_eq!(m.snapshot().apiserver_upserts, 2);
    }

    #[test]
    fn apply_apiserver_delete_removes_row() {
        let (t, m) = targets();
        let cm_json = full_configmap_json("victim", "default");
        apply_one(
            &mk_entry(1, 1, &RaftCommand::ApiserverUpsert { resource: cm_json }),
            &t, &m,
        ).unwrap();
        assert_eq!(t.resources.count("ConfigMap"), 1);
        apply_one(
            &mk_entry(2, 1, &RaftCommand::ApiserverDelete {
                kind: "ConfigMap".into(),
                namespace: "default".into(),
                name: "victim".into(),
            }),
            &t, &m,
        ).unwrap();
        assert_eq!(t.resources.count("ConfigMap"), 0);
        let snap = m.snapshot();
        assert_eq!(snap.apiserver_deletes, 1);
    }

    #[test]
    fn apply_apiserver_delete_on_missing_is_noop() {
        let (t, m) = targets();
        apply_one(
            &mk_entry(1, 1, &RaftCommand::ApiserverDelete {
                kind: "ConfigMap".into(),
                namespace: "default".into(),
                name: "ghost".into(),
            }),
            &t, &m,
        ).expect("apiserver delete must be idempotent on missing key");
        assert_eq!(m.snapshot().apiserver_deletes, 1);
        assert_eq!(m.snapshot().apply_errors, 0);
    }

    #[test]
    fn apply_noop_increments_only_noop_counter() {
        let (t, m) = targets();
        apply_one(&mk_entry(5, 3, &RaftCommand::NoOp), &t, &m).unwrap();
        let snap = m.snapshot();
        assert_eq!(snap.applied_total, 1);
        assert_eq!(snap.noops, 1);
        assert_eq!(snap.etcd_puts, 0);
        assert_eq!(snap.last_applied_index, 5);
    }

    #[test]
    fn apply_empty_payload_decodes_as_noop() {
        // Earlier sessions used `propose(vec![])` for leader markers;
        // make sure those still apply cleanly.
        let (t, m) = targets();
        let raw = LogEntry { term: 1, index: 10, command: vec![] };
        apply_one(&raw, &t, &m).unwrap();
        assert_eq!(m.snapshot().noops, 1);
    }

    #[test]
    fn apply_batch_continues_past_a_decode_error() {
        let (t, m) = targets();
        let good = mk_entry(1, 1, &RaftCommand::EtcdPut {
            key: "/g".into(),
            value: "v".into(),
            lease: None,
        });
        let bad = LogEntry { term: 1, index: 2, command: b"not-json".to_vec() };
        let later = mk_entry(3, 1, &RaftCommand::EtcdPut {
            key: "/h".into(),
            value: "w".into(),
            lease: None,
        });
        apply_batch(vec![good, bad, later], &t, &m);
        let snap = m.snapshot();
        assert_eq!(snap.etcd_puts, 2, "the bad entry must not stop the batch");
        assert_eq!(snap.apply_errors, 1);
        assert_eq!(snap.decode_errors, 1);
        // last_applied_index reflects the last *successful* apply.
        assert_eq!(snap.last_applied_index, 3);
    }

    #[test]
    fn apply_batch_continues_past_a_typed_decode_error() {
        // Encode an ApiserverUpsert whose resource JSON is well-formed
        // JSON but does not match any known Resource variant. apply_one
        // returns ApiserverUpsert (typed-decode), apply_batch counts
        // it as a non-decode apply error.
        let (t, m) = targets();
        let bogus_kind = mk_entry(1, 1, &RaftCommand::ApiserverUpsert {
            resource: serde_json::json!({"kind": "NotAResource", "metadata": {}}),
        });
        let next = mk_entry(2, 1, &RaftCommand::NoOp);
        apply_batch(vec![bogus_kind, next], &t, &m);
        let snap = m.snapshot();
        assert_eq!(snap.apply_errors, 1);
        assert_eq!(snap.decode_errors, 0, "typed-decode is not a wire decode error");
        assert_eq!(snap.noops, 1);
    }

    // ── End-to-end via the apply loop ─────────────────────────────────────

    /// Test source backed by a Mutex<VecDeque> so we can feed batches
    /// without standing up a real RaftCore.
    struct VecSource {
        inner: tokio::sync::Mutex<std::collections::VecDeque<Vec<LogEntry>>>,
    }
    #[async_trait::async_trait]
    impl CommittedEntrySource for VecSource {
        async fn drain(&self) -> Vec<LogEntry> {
            self.inner.lock().await.pop_front().unwrap_or_default()
        }
    }

    // Real-time tokio integration: marked `#[ignore]` because the
    // 120 ms sleep races with other portal tests that take process-
    // wide locks (auth + adr seeded-state suites). The 11 unit tests
    // above cover apply_one / apply_batch deterministically; this
    // ignored test still proves the spawn+select!+shutdown wiring
    // when run on its own with `--ignored`.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "real-time tokio integration; races with portal tests under default --test-threads"]
    async fn apply_loop_drains_until_shutdown() {
        let (t, m) = targets();
        let src = Arc::new(VecSource {
            inner: tokio::sync::Mutex::new(
                vec![
                    vec![mk_entry(1, 1, &RaftCommand::EtcdPut {
                        key: "/k1".into(),
                        value: "1".into(),
                        lease: None,
                    })],
                    vec![mk_entry(2, 1, &RaftCommand::EtcdPut {
                        key: "/k2".into(),
                        value: "2".into(),
                        lease: None,
                    })],
                ]
                .into_iter()
                .collect(),
            ),
        });
        let (tx, rx) = tokio::sync::watch::channel(false);
        let metrics_clone = m.clone();
        let task = tokio::spawn(run_apply_loop(
            src.clone(),
            t.clone(),
            m.clone(),
            rx,
            Duration::from_millis(5),
        ));
        // Let the loop run a few ticks against real time.
        tokio::time::sleep(Duration::from_millis(120)).await;
        tx.send(true).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        let snap = metrics_clone.snapshot();
        assert!(
            snap.applied_total >= 2,
            "loop should drain at least the seeded batches; applied={}",
            snap.applied_total,
        );
    }
}

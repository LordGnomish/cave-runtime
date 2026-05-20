// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bridge between cave-ha's concrete Raft node and the abstract
//! `cave_kernel::consensus` trait surface (sweep-002 F2-A).
//!
//! Adoption goals:
//!
//!   * Downstream crates (cave-etcd, cave-apiserver) interact with the Raft
//!     node through `cave_kernel::consensus::{LogStore, StateMachine,
//!     RaftHandle}` traits — the same traits every other consensus-aware crate
//!     uses. cave-ha is no longer special.
//!
//!   * The local cave-ha types remain free to carry implementation detail
//!     (`EntryType`, `MembershipConfig`, etc.) that the abstract surface does
//!     not need. This module performs the small projections.
//!
//! What lives here:
//!
//!   * [`to_kernel_entry`] / [`from_kernel_entry`] — projection between
//!     cave-ha's `LogEntry` (typed `EntryType`) and the kernel's untagged
//!     `LogEntry { index, term, data }`.
//!   * [`map_ha_error`] — `HaError` → `ConsensusError` translation. Maps
//!     `NotLeader` to the kernel's leader-aware variant; everything else
//!     funnels into `Storage`/`Aborted` per the kernel contract.
//!   * [`KernelLogStore`] — wraps `Arc<tokio::sync::Mutex<MemLog>>` and
//!     implements `cave_kernel::consensus::LogStore`. Uses async-aware
//!     locking so the kernel trait's `async fn append/get/last_index/...`
//!     surface is honored without blocking.
//!   * [`KernelRaftHandle`] — wraps the cave-ha `RaftHandle` and implements
//!     `cave_kernel::consensus::RaftHandle`. Translates `propose/read_index`
//!     directly; `leader()` extracts a `LeaderInfo` from `status()`;
//!     `node_id()` stringifies the cave-ha numeric NodeId.
//!
//! No duplicate Raft logic lives here — only projections. The kernel trait is
//! the canonical surface; cave-ha implements it.

use crate::error::HaError;
use crate::raft::log::{LogEntry as HaLogEntry, MemLog};
use crate::raft::node::RaftHandle as HaRaftHandle;
use crate::raft::types::{EntryType, NodeId as HaNodeId, Role as HaRole};
use async_trait::async_trait;
use cave_kernel::consensus::{
    ConsensusError, ConsensusResult, LeaderInfo, LogEntry as KernelLogEntry, LogIndex, LogStore,
    NodeId as KernelNodeId, RaftHandle as KernelRaftHandleTrait, Role as KernelRole,
};
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Conversions ──────────────────────────────────────────────────────────────

/// Project a cave-ha `LogEntry` to the kernel's untagged shape. The
/// `entry_type` discriminator is dropped — the kernel surface treats every
/// entry as opaque bytes (membership/barrier handling is internal to cave-ha).
pub fn to_kernel_entry(entry: &HaLogEntry) -> KernelLogEntry {
    KernelLogEntry {
        index: entry.index,
        term: entry.term,
        data: entry.data.clone(),
    }
}

/// Lift a kernel `LogEntry` back into a cave-ha `Normal` entry. Used by
/// `KernelLogStore::append` when external callers (cave-etcd, cave-apiserver)
/// stage entries through the kernel trait — they cannot signal Membership or
/// Barrier kinds, which are produced internally by the cave-ha Raft loop.
pub fn from_kernel_entry(entry: &KernelLogEntry) -> HaLogEntry {
    HaLogEntry {
        index: entry.index,
        term: entry.term,
        entry_type: EntryType::Normal,
        data: entry.data.clone(),
    }
}

/// Translate cave-ha `HaError` to the kernel's `ConsensusError`.
pub fn map_ha_error(e: HaError) -> ConsensusError {
    match e {
        HaError::NotLeader { leader_id } => {
            ConsensusError::NotLeader(leader_id.map(|id| id.to_string()))
        }
        HaError::LogCompacted { requested, .. } => ConsensusError::LogNotFound(requested),
        HaError::Storage(s) => ConsensusError::Storage(s),
        HaError::Transport(s) => ConsensusError::Transport(s),
        HaError::Snapshot(s) => ConsensusError::Storage(format!("snapshot: {s}")),
        HaError::Serialization(s) => ConsensusError::Storage(format!("serde: {s}")),
        HaError::Io(s) => ConsensusError::Storage(format!("io: {s}")),
        HaError::Raft(s) => ConsensusError::Aborted(s),
        HaError::Shutdown => ConsensusError::Aborted("shutdown".into()),
        HaError::Timeout => ConsensusError::Aborted("timeout".into()),
        HaError::ProposalDropped => ConsensusError::Aborted("proposal dropped".into()),
        HaError::TransferInProgress => ConsensusError::Aborted("transfer in progress".into()),
        HaError::IsLearner => ConsensusError::Aborted("node is learner".into()),
        HaError::MembershipChangePending => {
            ConsensusError::Aborted("membership change pending".into())
        }
        HaError::NodeNotFound(id) => ConsensusError::Aborted(format!("node {id} not found")),
        HaError::NoQuorum => ConsensusError::Aborted("no quorum".into()),
        HaError::Dr(s) => ConsensusError::Aborted(format!("dr: {s}")),
    }
}

/// Project cave-ha's `Role` (which has an extra `PreCandidate` phase) onto
/// the kernel's three-state `Role`. PreCandidate is reported as `Candidate`
/// to the kernel — the distinction is internal to the pre-vote optimization.
pub fn to_kernel_role(role: &HaRole) -> KernelRole {
    match role {
        HaRole::Follower => KernelRole::Follower,
        HaRole::PreCandidate | HaRole::Candidate => KernelRole::Candidate,
        HaRole::Leader => KernelRole::Leader,
    }
}

/// Stringify a cave-ha numeric NodeId for the kernel surface. The kernel
/// uses transport-neutral `String` IDs.
pub fn to_kernel_node_id(id: HaNodeId) -> KernelNodeId {
    id.to_string()
}

// ── LogStore adapter ─────────────────────────────────────────────────────────

/// Adapts an `Arc<Mutex<MemLog>>` to the kernel's `LogStore` trait.
///
/// The cave-ha `MemLog` is sync; we wrap it in a tokio `Mutex` so the
/// kernel's async API can be honored without blocking the executor on lock
/// acquisition. Callers that already share a `MemLog` from the cave-ha node
/// loop should hand a clone of the same `Arc` to keep state consistent.
#[derive(Clone)]
pub struct KernelLogStore {
    inner: Arc<Mutex<MemLog>>,
}

impl KernelLogStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MemLog::new())),
        }
    }

    pub fn from_arc(inner: Arc<Mutex<MemLog>>) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &Arc<Mutex<MemLog>> {
        &self.inner
    }
}

impl Default for KernelLogStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LogStore for KernelLogStore {
    async fn append(&self, entries: &[KernelLogEntry]) -> ConsensusResult<()> {
        let mut log = self.inner.lock().await;
        let lifted: Vec<HaLogEntry> = entries.iter().map(from_kernel_entry).collect();
        log.append(lifted);
        Ok(())
    }

    async fn get(&self, index: LogIndex) -> ConsensusResult<Option<KernelLogEntry>> {
        let log = self.inner.lock().await;
        match log.entry(index) {
            Ok(e) => Ok(Some(to_kernel_entry(e))),
            Err(HaError::LogCompacted { .. }) => Ok(None),
            Err(HaError::Raft(_)) => Ok(None), // not-found surfaces as None per kernel contract
            Err(e) => Err(map_ha_error(e)),
        }
    }

    async fn last_index(&self) -> ConsensusResult<LogIndex> {
        let log = self.inner.lock().await;
        Ok(log.last_index())
    }

    async fn truncate_after(&self, index: LogIndex) -> ConsensusResult<()> {
        let mut log = self.inner.lock().await;
        log.truncate_to(index);
        Ok(())
    }
}

// ── RaftHandle adapter ───────────────────────────────────────────────────────

/// Wrap a cave-ha `RaftHandle` so downstream code can hold it as an
/// `Arc<dyn cave_kernel::consensus::RaftHandle>`. Cheap to clone (the inner
/// handle is itself Clone).
#[derive(Clone)]
pub struct KernelRaftHandle {
    inner: HaRaftHandle,
}

impl KernelRaftHandle {
    pub fn new(inner: HaRaftHandle) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &HaRaftHandle {
        &self.inner
    }
}

#[async_trait]
impl KernelRaftHandleTrait for KernelRaftHandle {
    async fn propose(&self, data: Vec<u8>) -> ConsensusResult<LogIndex> {
        self.inner.propose(data).await.map_err(map_ha_error)
    }

    async fn read_index(&self) -> ConsensusResult<LogIndex> {
        self.inner.read_index().await.map_err(map_ha_error)
    }

    async fn leader(&self) -> ConsensusResult<LeaderInfo> {
        let status = self.inner.status().await.map_err(map_ha_error)?;
        // cave-ha's NodeStatus carries `role: String`; reverse-parse to the
        // canonical kernel Role. Anything we don't recognize collapses to
        // Follower.
        let role = match status.role.as_str() {
            "Leader" => KernelRole::Leader,
            "Candidate" | "PreCandidate" => KernelRole::Candidate,
            _ => KernelRole::Follower,
        };
        Ok(LeaderInfo {
            leader: status.leader_id.map(to_kernel_node_id),
            term: status.term,
            role,
        })
    }

    async fn node_id(&self) -> KernelNodeId {
        to_kernel_node_id(self.inner.node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raft::log::LogEntry as HaLogEntry;
    use crate::raft::state_machine::{KvStateMachine, NoopStateMachine};
    use crate::raft::types::EntryType;
    use cave_kernel::consensus::StateMachine;

    // ── Conversions ──────────────────────────────────────────────────────────

    #[test]
    fn to_kernel_entry_preserves_index_term_and_data() {
        let ha = HaLogEntry::new_normal(7, 3, b"hello".to_vec());
        let k = to_kernel_entry(&ha);
        assert_eq!(k.index, 7);
        assert_eq!(k.term, 3);
        assert_eq!(k.data, b"hello".to_vec());
    }

    #[test]
    fn to_kernel_entry_drops_entry_type_discriminator() {
        let barrier = HaLogEntry::new_barrier(11, 2);
        let k = to_kernel_entry(&barrier);
        assert_eq!(k.index, 11);
        assert_eq!(k.term, 2);
        assert!(
            k.data.is_empty(),
            "barrier data is empty by construction; kernel surface untagged"
        );
    }

    #[test]
    fn from_kernel_entry_lifts_to_normal_variant() {
        let k = KernelLogEntry {
            index: 4,
            term: 1,
            data: b"abc".to_vec(),
        };
        let ha = from_kernel_entry(&k);
        assert_eq!(ha.index, 4);
        assert_eq!(ha.term, 1);
        assert_eq!(
            ha.entry_type,
            EntryType::Normal,
            "kernel-staged entries are always Normal at the bridge"
        );
        assert_eq!(ha.data, b"abc".to_vec());
    }

    #[test]
    fn round_trip_kernel_entry_preserves_payload() {
        let original = HaLogEntry::new_normal(99, 41, b"payload".to_vec());
        let kernel = to_kernel_entry(&original);
        let back = from_kernel_entry(&kernel);
        assert_eq!(back.index, original.index);
        assert_eq!(back.term, original.term);
        assert_eq!(back.data, original.data);
    }

    #[test]
    fn map_ha_error_not_leader_carries_leader_id_string() {
        let e = HaError::NotLeader { leader_id: Some(7) };
        match map_ha_error(e) {
            ConsensusError::NotLeader(Some(id)) => assert_eq!(id, "7"),
            other => panic!("expected NotLeader(Some), got {other:?}"),
        }
    }

    #[test]
    fn map_ha_error_not_leader_handles_unknown_leader() {
        let e = HaError::NotLeader { leader_id: None };
        assert!(matches!(map_ha_error(e), ConsensusError::NotLeader(None)));
    }

    #[test]
    fn map_ha_error_log_compacted_surfaces_log_not_found() {
        let e = HaError::LogCompacted {
            requested: 42,
            snapshot: 100,
        };
        assert!(matches!(map_ha_error(e), ConsensusError::LogNotFound(42)));
    }

    #[test]
    fn map_ha_error_storage_passes_through() {
        let e = HaError::Storage("disk full".into());
        match map_ha_error(e) {
            ConsensusError::Storage(s) => assert!(s.contains("disk full")),
            other => panic!("expected Storage, got {other:?}"),
        }
    }

    #[test]
    fn map_ha_error_transport_passes_through() {
        let e = HaError::Transport("rst".into());
        match map_ha_error(e) {
            ConsensusError::Transport(s) => assert!(s.contains("rst")),
            other => panic!("expected Transport, got {other:?}"),
        }
    }

    #[test]
    fn map_ha_error_shutdown_aborts() {
        match map_ha_error(HaError::Shutdown) {
            ConsensusError::Aborted(s) => assert!(s.contains("shutdown")),
            other => panic!("expected Aborted, got {other:?}"),
        }
    }

    #[test]
    fn map_ha_error_proposal_dropped_aborts() {
        match map_ha_error(HaError::ProposalDropped) {
            ConsensusError::Aborted(s) => assert!(s.to_lowercase().contains("proposal")),
            other => panic!("expected Aborted, got {other:?}"),
        }
    }

    #[test]
    fn to_kernel_role_collapses_pre_candidate() {
        assert!(matches!(
            to_kernel_role(&HaRole::Follower),
            KernelRole::Follower
        ));
        assert!(matches!(
            to_kernel_role(&HaRole::Candidate),
            KernelRole::Candidate
        ));
        assert!(
            matches!(to_kernel_role(&HaRole::PreCandidate), KernelRole::Candidate),
            "PreCandidate is an internal phase; kernel surface sees Candidate"
        );
        assert!(matches!(
            to_kernel_role(&HaRole::Leader),
            KernelRole::Leader
        ));
    }

    #[test]
    fn to_kernel_node_id_stringifies_numeric_id() {
        assert_eq!(to_kernel_node_id(42), "42");
        assert_eq!(to_kernel_node_id(0), "0");
        assert_eq!(to_kernel_node_id(u64::MAX), u64::MAX.to_string());
    }

    // ── KernelLogStore conformance ───────────────────────────────────────────

    #[tokio::test]
    async fn log_store_append_then_get_round_trips() {
        let store = KernelLogStore::new();
        let entries = vec![
            KernelLogEntry {
                index: 1,
                term: 1,
                data: b"a".to_vec(),
            },
            KernelLogEntry {
                index: 2,
                term: 1,
                data: b"b".to_vec(),
            },
        ];
        store.append(&entries).await.unwrap();
        let got = store.get(1).await.unwrap().expect("entry 1 present");
        assert_eq!(got.data, b"a".to_vec());
        assert_eq!(got.term, 1);
    }

    #[tokio::test]
    async fn log_store_last_index_advances_with_appends() {
        let store = KernelLogStore::new();
        assert_eq!(
            store.last_index().await.unwrap(),
            0,
            "empty log has last_index 0"
        );
        store
            .append(&[KernelLogEntry {
                index: 1,
                term: 1,
                data: vec![],
            }])
            .await
            .unwrap();
        assert_eq!(store.last_index().await.unwrap(), 1);
        store
            .append(&[KernelLogEntry {
                index: 2,
                term: 1,
                data: vec![],
            }])
            .await
            .unwrap();
        assert_eq!(store.last_index().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn log_store_get_missing_returns_none() {
        let store = KernelLogStore::new();
        store
            .append(&[KernelLogEntry {
                index: 1,
                term: 1,
                data: vec![],
            }])
            .await
            .unwrap();
        // Asking for an index past the last → kernel contract says Ok(None).
        let got = store.get(99).await.unwrap();
        assert!(got.is_none(), "missing index → None, not error");
    }

    #[tokio::test]
    async fn log_store_truncate_after_drops_suffix() {
        let store = KernelLogStore::new();
        store
            .append(&[
                KernelLogEntry {
                    index: 1,
                    term: 1,
                    data: vec![1],
                },
                KernelLogEntry {
                    index: 2,
                    term: 1,
                    data: vec![2],
                },
                KernelLogEntry {
                    index: 3,
                    term: 1,
                    data: vec![3],
                },
            ])
            .await
            .unwrap();
        store.truncate_after(1).await.unwrap();
        assert_eq!(
            store.last_index().await.unwrap(),
            1,
            "truncate_after(1) keeps entries [0..=1]"
        );
        assert!(store.get(2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn log_store_append_then_truncate_then_append_re_uses_indices() {
        // Mirrors upstream Raft conflict-resolution flow: follower truncates a
        // diverging suffix, then accepts the leader's new entries at the same
        // indices.
        let store = KernelLogStore::new();
        store
            .append(&[
                KernelLogEntry {
                    index: 1,
                    term: 1,
                    data: vec![1],
                },
                KernelLogEntry {
                    index: 2,
                    term: 1,
                    data: vec![2],
                },
            ])
            .await
            .unwrap();
        store.truncate_after(1).await.unwrap();
        store
            .append(&[KernelLogEntry {
                index: 2,
                term: 2,
                data: vec![20],
            }])
            .await
            .unwrap();
        let e = store.get(2).await.unwrap().expect("entry 2 present");
        assert_eq!(
            e.term, 2,
            "post-truncation append takes the new leader's term at index 2"
        );
        assert_eq!(e.data, vec![20]);
    }

    #[tokio::test]
    async fn log_store_clone_shares_storage() {
        let store = KernelLogStore::new();
        let store2 = store.clone();
        store
            .append(&[KernelLogEntry {
                index: 1,
                term: 1,
                data: vec![1],
            }])
            .await
            .unwrap();
        // Second handle observes the same backing store.
        assert_eq!(store2.last_index().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn log_store_from_arc_shares_existing_memlog() {
        let mem = Arc::new(Mutex::new(MemLog::new()));
        let store = KernelLogStore::from_arc(mem.clone());
        store
            .append(&[KernelLogEntry {
                index: 1,
                term: 1,
                data: vec![1],
            }])
            .await
            .unwrap();
        // Inspect via the original Arc — same data.
        let inner = mem.lock().await;
        assert_eq!(inner.last_index(), 1);
    }

    // ── State machine conformance (kernel trait directly) ────────────────────

    #[tokio::test]
    async fn noop_state_machine_apply_returns_empty() {
        let sm = NoopStateMachine::default();
        let entry = KernelLogEntry {
            index: 1,
            term: 1,
            data: b"anything".to_vec(),
        };
        assert!(sm.apply(&entry).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn noop_state_machine_snapshot_and_restore_are_idempotent() {
        let sm = NoopStateMachine::default();
        let snap = sm.snapshot().await.unwrap();
        sm.restore(&snap).await.unwrap();
        // No-op machine — round-trip must succeed without state.
    }

    #[tokio::test]
    async fn kv_state_machine_apply_set_persists_value() {
        use serde_json::json;
        let sm = KvStateMachine::new();
        let cmd = json!({ "Set": { "key": "k", "value": "v" } });
        let entry = KernelLogEntry {
            index: 1,
            term: 1,
            data: serde_json::to_vec(&cmd).unwrap(),
        };
        sm.apply(&entry).await.unwrap();
        assert_eq!(sm.get("k").await, Some("v".to_string()));
    }

    #[tokio::test]
    async fn kv_state_machine_apply_delete_removes_value() {
        use serde_json::json;
        let sm = KvStateMachine::new();
        let set = serde_json::to_vec(&json!({ "Set": { "key": "k", "value": "v" } })).unwrap();
        let del = serde_json::to_vec(&json!({ "Delete": { "key": "k" } })).unwrap();
        sm.apply(&KernelLogEntry {
            index: 1,
            term: 1,
            data: set,
        })
        .await
        .unwrap();
        sm.apply(&KernelLogEntry {
            index: 2,
            term: 1,
            data: del,
        })
        .await
        .unwrap();
        assert_eq!(sm.get("k").await, None);
    }

    #[tokio::test]
    async fn kv_state_machine_apply_empty_data_is_noop() {
        let sm = KvStateMachine::new();
        let entry = KernelLogEntry {
            index: 1,
            term: 1,
            data: vec![],
        };
        assert!(
            sm.apply(&entry).await.unwrap().is_empty(),
            "empty data is a no-op (matches Barrier semantics)"
        );
    }

    #[tokio::test]
    async fn kv_state_machine_apply_invalid_data_yields_storage_error() {
        let sm = KvStateMachine::new();
        let entry = KernelLogEntry {
            index: 1,
            term: 1,
            data: b"not-json".to_vec(),
        };
        match sm.apply(&entry).await.unwrap_err() {
            ConsensusError::Storage(_) => {} // expected
            other => panic!("expected Storage error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn kv_state_machine_snapshot_restore_round_trip() {
        use serde_json::json;
        let sm = KvStateMachine::new();
        for (i, (k, v)) in [("a", "1"), ("b", "2"), ("c", "3")].iter().enumerate() {
            let cmd = serde_json::to_vec(&json!({ "Set": { "key": k, "value": v } })).unwrap();
            sm.apply(&KernelLogEntry {
                index: (i as u64) + 1,
                term: 1,
                data: cmd,
            })
            .await
            .unwrap();
        }
        let snapshot = sm.snapshot().await.unwrap();
        let sm2 = KvStateMachine::new();
        sm2.restore(&snapshot).await.unwrap();
        assert_eq!(sm2.get("a").await, Some("1".into()));
        assert_eq!(sm2.get("b").await, Some("2".into()));
        assert_eq!(sm2.get("c").await, Some("3".into()));
    }

    #[tokio::test]
    async fn kv_state_machine_restore_invalid_yields_storage_error() {
        let sm = KvStateMachine::new();
        match sm.restore(b"not-json").await.unwrap_err() {
            ConsensusError::Storage(_) => {}
            other => panic!("expected Storage error, got {other:?}"),
        }
    }

    // ── Trait-object usability via kernel surface ────────────────────────────

    #[tokio::test]
    async fn kernel_trait_objects_compose() {
        // A downstream crate should be able to hold the kernel trait object
        // type-erased and never reference cave-ha types directly. This test
        // proves that compiles + runs end-to-end.
        let sm: Arc<dyn StateMachine> = Arc::new(KvStateMachine::new());
        let store: Arc<dyn LogStore> = Arc::new(KernelLogStore::new());

        store
            .append(&[KernelLogEntry {
                index: 1,
                term: 1,
                data: vec![],
            }])
            .await
            .unwrap();
        let entry = store.get(1).await.unwrap().expect("entry");
        sm.apply(&entry).await.unwrap();
    }

    // ── End-to-end: spawn a real Raft node and drive it through the kernel
    //    handle. Single-node cluster commits proposals immediately, so the
    //    test is deterministic without needing transport plumbing.

    use crate::config::NodeConfig;
    use crate::metrics::Metrics;
    use crate::raft::node::RaftNode;
    use crate::raft::types::NodeInfo;
    use crate::transport::memory::MemNetwork;
    use prometheus_client::registry::Registry;

    fn cfg(id: HaNodeId) -> NodeConfig {
        NodeConfig {
            id,
            election_timeout_min: 5,
            election_timeout_max: 10,
            heartbeat_interval: 2,
            pre_vote: true,
            ..Default::default()
        }
    }

    /// Spawn a single-node Raft cluster wired through the kernel adapter.
    /// Returns the cave-ha handle (for shutdown) and the kernel adapter.
    async fn spawn_single(
        id: HaNodeId,
        sm: Arc<dyn StateMachine>,
    ) -> (HaRaftHandle, KernelRaftHandle) {
        let net = Arc::new(MemNetwork::new());
        let (transport, mut rx) = net.register(id);
        let mut registry = Registry::default();
        let metrics = Metrics::new(&mut registry);
        let inner = RaftNode::spawn(
            cfg(id),
            vec![NodeInfo {
                id,
                addr: format!("mem:{id}"),
                is_learner: false,
            }],
            Arc::new(transport),
            sm,
            metrics,
        );
        let msg_tx = inner.msg_tx.clone();
        tokio::spawn(async move {
            while let Some((from, msg)) = rx.recv().await {
                if msg_tx.send((from, msg)).is_err() {
                    break;
                }
            }
        });
        let kernel = KernelRaftHandle::new(inner.clone());
        (inner, kernel)
    }

    /// Spin until the node self-elects (single-node cluster reaches Leader
    /// in the first election cycle).
    async fn wait_until_leader(h: &HaRaftHandle) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if tokio::time::Instant::now() >= deadline {
                panic!("timed out waiting for self-election");
            }
            if let Ok(s) = h.status().await {
                if s.role == "Leader" {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kernel_handle_node_id_matches_inner_node() {
        let sm: Arc<dyn StateMachine> = Arc::new(NoopStateMachine);
        let (inner, kernel) = spawn_single(7, sm).await;
        let kernel_dyn: Arc<dyn KernelRaftHandleTrait> = Arc::new(kernel);
        assert_eq!(kernel_dyn.node_id().await, "7");
        inner.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kernel_handle_leader_returns_leader_info_after_election() {
        let sm: Arc<dyn StateMachine> = Arc::new(NoopStateMachine);
        let (inner, kernel) = spawn_single(1, sm).await;
        wait_until_leader(&inner).await;
        let info = kernel.leader().await.unwrap();
        assert!(info.term >= 1, "leader info carries the elected term");
        assert!(
            matches!(info.role, KernelRole::Leader),
            "single-node cluster elects itself"
        );
        assert_eq!(
            info.leader,
            Some("1".to_string()),
            "leader is reported as the kernel-stringified node id"
        );
        inner.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kernel_handle_propose_returns_committed_index() {
        let sm: Arc<dyn StateMachine> = Arc::new(KvStateMachine::new());
        let (inner, kernel) = spawn_single(1, sm).await;
        wait_until_leader(&inner).await;
        let cmd = serde_json::to_vec(&serde_json::json!({ "Set": { "key": "k", "value": "v" } }))
            .unwrap();
        let idx = kernel
            .propose(cmd)
            .await
            .expect("propose succeeds on leader");
        assert!(idx >= 1, "propose returns a committed log index");
        inner.shutdown().await;
    }

    /// Propose path is reachable repeatedly through the kernel adapter — the
    /// adapter's mpsc-based wiring does not leak resources across calls.
    /// (Note: the cave-ha single-node `read_index` path is known to block on
    /// quorum-ack accounting and is exercised in cave-ha's multi-node
    /// integration tests rather than here. The kernel adapter's `read_index`
    /// implementation itself is a one-line forward + error map.)
    #[tokio::test(flavor = "multi_thread")]
    async fn kernel_handle_propose_repeated_calls_advance_index() {
        let sm: Arc<dyn StateMachine> = Arc::new(NoopStateMachine);
        let (inner, kernel) = spawn_single(1, sm).await;
        wait_until_leader(&inner).await;
        let i1 = kernel.propose(b"a".to_vec()).await.unwrap();
        let i2 = kernel.propose(b"b".to_vec()).await.unwrap();
        let i3 = kernel.propose(b"c".to_vec()).await.unwrap();
        assert!(i2 > i1, "second propose returns higher index");
        assert!(i3 > i2, "third propose returns higher index");
        inner.shutdown().await;
    }

    /// Cloning the kernel handle yields handles that share the same inner
    /// node (verifies `KernelRaftHandle: Clone` by way of the inner
    /// cave-ha `RaftHandle: Clone`).
    #[tokio::test(flavor = "multi_thread")]
    async fn kernel_handle_clone_shares_inner_node() {
        let sm: Arc<dyn StateMachine> = Arc::new(NoopStateMachine);
        let (inner, kernel) = spawn_single(3, sm).await;
        let kernel2 = kernel.clone();
        assert_eq!(kernel.node_id().await, kernel2.node_id().await);
        assert_eq!(kernel.node_id().await, "3");
        inner.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kernel_handle_propose_after_shutdown_yields_aborted() {
        let sm: Arc<dyn StateMachine> = Arc::new(NoopStateMachine);
        let (inner, kernel) = spawn_single(1, sm).await;
        inner.shutdown().await;
        // Give the node loop a moment to exit.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let err = kernel.propose(b"data".to_vec()).await.unwrap_err();
        match err {
            ConsensusError::Aborted(_) => {} // expected
            other => panic!("expected Aborted on shutdown, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dyn_raft_handle_alias_is_arc_dyn() {
        // Smoke check: kernel exposes `DynRaftHandle = Arc<dyn RaftHandle>`.
        // Our adapter must satisfy that bound.
        fn requires_dyn(_: cave_kernel::consensus::DynRaftHandle) {}
        let sm: Arc<dyn StateMachine> = Arc::new(NoopStateMachine);
        let (inner, kernel) = spawn_single(2, sm).await;
        let dyn_handle: cave_kernel::consensus::DynRaftHandle = Arc::new(kernel);
        requires_dyn(dyn_handle);
        inner.shutdown().await;
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Optional Raft seam for the etcd write-path handlers.
//!
//! Single-node deployments mutate [`crate::store::KvStore`] directly
//! and never look at this trait. Multi-node deployments install a
//! [`RaftBridge`] implementation that propose-and-waits through
//! `cave-runtime`'s Raft core; writes then only return success after
//! the entry has been committed + applied on this node.
//!
//! Dependency direction
//! --------------------
//!
//! `cave-etcd` does **not** depend on `cave-runtime` — the host
//! provides the bridge implementation when it builds the etcd
//! router. This trait is therefore intentionally minimal: it speaks
//! in `(key, value, lease)` triples rather than `RaftCommand` so the
//! command schema lives entirely in `cave-runtime`.

use async_trait::async_trait;
use std::sync::Arc;

/// Errors the bridge surfaces back to HTTPS handlers. Each variant
/// maps to a specific status code the handler can emit.
#[derive(Debug, thiserror::Error)]
pub enum RaftBridgeError {
    /// The local node is not the current Raft leader. Includes the
    /// leader's HTTPS URL (if known) so the handler can return a
    /// `503 + Location:` header that the client can retry against.
    #[error("not leader; leader_url={leader_url:?}")]
    NotLeader { leader_url: Option<String> },
    /// Deadline elapsed waiting for the proposed entry to commit and
    /// apply on this node.
    #[error("timeout waiting for commit+apply")]
    Timeout,
    /// Anything else (encode error, propose error, channel closed).
    #[error("{0}")]
    Internal(String),
}

/// The contract cave-etcd's write handlers consult when configured
/// for multi-node Raft mode. Implementations live in `cave-runtime`.
#[async_trait]
pub trait RaftBridge: Send + Sync + std::fmt::Debug {
    /// True iff the local node is the current Raft leader. Returning
    /// `false` causes the handler to skip propose and respond with a
    /// `503` + leader URL hint.
    fn is_leader(&self) -> bool;

    /// Best-effort leader HTTPS URL. None when the local node has not
    /// observed a leader yet (e.g. during a partition).
    fn leader_url(&self) -> Option<String>;

    /// Propose an etcd PUT and wait for it to be applied on this
    /// node. Returns `Ok(())` after `apply_one` has written the
    /// key/value into the local KvStore.
    async fn propose_put(
        &self,
        key: String,
        value: String,
        lease: Option<i64>,
    ) -> Result<(), RaftBridgeError>;

    /// Propose an etcd DELETE / range-DELETE. Returns `Ok(())` after
    /// apply. `range_end = None` means single-key delete.
    async fn propose_delete(
        &self,
        key: String,
        range_end: Option<String>,
    ) -> Result<(), RaftBridgeError>;
}

/// Shared alias for the trait object used by handler state.
pub type SharedRaftBridge = Arc<dyn RaftBridge>;

#[cfg(test)]
pub mod test_doubles {
    //! Test fakes for unit-testing the etcd write handlers.
    //! Production callers use `cave_runtime`'s real bridge.

    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Records every propose call and lets tests assert what arrived.
    #[derive(Debug)]
    pub struct RecordingBridge {
        pub is_leader: AtomicBool,
        pub leader_url: Mutex<Option<String>>,
        pub put_calls: Mutex<Vec<(String, String, Option<i64>)>>,
        pub delete_calls: Mutex<Vec<(String, Option<String>)>>,
        pub force_timeout: AtomicBool,
        pub propose_count: AtomicUsize,
    }

    impl RecordingBridge {
        pub fn leader() -> Self {
            Self {
                is_leader: AtomicBool::new(true),
                leader_url: Mutex::new(None),
                put_calls: Mutex::new(Vec::new()),
                delete_calls: Mutex::new(Vec::new()),
                force_timeout: AtomicBool::new(false),
                propose_count: AtomicUsize::new(0),
            }
        }
        pub fn follower(leader_url: Option<String>) -> Self {
            let me = Self::leader();
            me.is_leader.store(false, Ordering::Relaxed);
            *me.leader_url.lock().unwrap() = leader_url;
            me
        }
    }

    #[async_trait]
    impl RaftBridge for RecordingBridge {
        fn is_leader(&self) -> bool {
            self.is_leader.load(Ordering::Relaxed)
        }
        fn leader_url(&self) -> Option<String> {
            self.leader_url.lock().unwrap().clone()
        }
        async fn propose_put(
            &self,
            key: String,
            value: String,
            lease: Option<i64>,
        ) -> Result<(), RaftBridgeError> {
            self.propose_count.fetch_add(1, Ordering::Relaxed);
            if self.force_timeout.load(Ordering::Relaxed) {
                return Err(RaftBridgeError::Timeout);
            }
            if !self.is_leader() {
                return Err(RaftBridgeError::NotLeader {
                    leader_url: self.leader_url(),
                });
            }
            self.put_calls.lock().unwrap().push((key, value, lease));
            Ok(())
        }
        async fn propose_delete(
            &self,
            key: String,
            range_end: Option<String>,
        ) -> Result<(), RaftBridgeError> {
            self.propose_count.fetch_add(1, Ordering::Relaxed);
            if !self.is_leader() {
                return Err(RaftBridgeError::NotLeader {
                    leader_url: self.leader_url(),
                });
            }
            self.delete_calls.lock().unwrap().push((key, range_end));
            Ok(())
        }
    }
}

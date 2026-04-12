//! In-process transport for tests and single-binary deployments.
//!
//! A `MemNetwork` owns a shared router; each node gets a `MemTransport` handle.
//! Partitions can be injected by calling `partition(a, b)`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::debug;

use crate::error::{HaError, HaResult};
use crate::raft::messages::RaftMessage;
use crate::raft::types::NodeId;
use crate::transport::Transport;

/// Shared routing table and partition state.
pub struct MemNetwork {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    /// node_id → channel for delivering messages.
    senders: HashMap<NodeId, mpsc::UnboundedSender<(NodeId, RaftMessage)>>,
    /// Directed partition: if (a, b) is in this set, messages from a to b are dropped.
    partitions: HashSet<(NodeId, NodeId)>,
    /// Drop rate (0.0 = no drops, 1.0 = drop all).  Useful for flaky network tests.
    drop_rate: f64,
}

impl Default for MemNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl MemNetwork {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                senders: HashMap::new(),
                partitions: HashSet::new(),
                drop_rate: 0.0,
            })),
        }
    }

    /// Register a node and return its `MemTransport`.
    /// The caller is responsible for delivering messages from the returned receiver
    /// to the Raft node's `msg_tx`.
    pub fn register(
        &self,
        id: NodeId,
    ) -> (MemTransport, mpsc::UnboundedReceiver<(NodeId, RaftMessage)>) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.inner.lock().unwrap().senders.insert(id, tx);
        (
            MemTransport { from: id, network: Arc::clone(&self.inner) },
            rx,
        )
    }

    /// Block messages from `a` to `b` (and optionally `b` to `a`).
    pub fn partition(&self, a: NodeId, b: NodeId, bidirectional: bool) {
        let mut g = self.inner.lock().unwrap();
        g.partitions.insert((a, b));
        if bidirectional {
            g.partitions.insert((b, a));
        }
    }

    /// Restore connectivity between `a` and `b`.
    pub fn heal(&self, a: NodeId, b: NodeId) {
        let mut g = self.inner.lock().unwrap();
        g.partitions.remove(&(a, b));
        g.partitions.remove(&(b, a));
    }

    /// Set a uniform packet drop rate (0.0–1.0) for chaos testing.
    pub fn set_drop_rate(&self, rate: f64) {
        self.inner.lock().unwrap().drop_rate = rate;
    }
}

/// Per-node handle — implements `Transport`.
pub struct MemTransport {
    from: NodeId,
    network: Arc<Mutex<Inner>>,
}

#[async_trait]
impl Transport for MemTransport {
    async fn send(&self, to: NodeId, msg: RaftMessage) -> HaResult<()> {
        let g = self.network.lock().unwrap();
        if g.partitions.contains(&(self.from, to)) {
            debug!(from = self.from, to, "message dropped (partition)");
            return Ok(());
        }
        if g.drop_rate > 0.0 {
            let sample: f64 = rand::random();
            if sample < g.drop_rate {
                debug!(from = self.from, to, "message dropped (chaos)");
                return Ok(());
            }
        }
        if let Some(tx) = g.senders.get(&to) {
            tx.send((self.from, msg)).map_err(|_| HaError::Transport(format!("node {to} gone")))?;
        } else {
            debug!(to, "no route to node");
        }
        Ok(())
    }
}

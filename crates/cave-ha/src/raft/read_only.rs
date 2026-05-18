// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashSet;
use tokio::sync::oneshot;
use crate::error::HaResult;
use crate::raft::types::{LogIndex, NodeId};

/// Read-only request mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    /// Safe linearizable reads via leader heartbeat round-trip.
    ReadIndex,
    /// Low-latency reads using leader lease (clock-based, no extra round-trip).
    LeaseRead,
}

/// A pending read-only request waiting for confirmation.
pub struct ReadRequest {
    pub id: u64,
    /// The commit index at the time the read was received.
    pub index: LogIndex,
    /// Peers that have sent heartbeat ACKs for this round.
    pub acks: HashSet<NodeId>,
    /// Channel to notify the waiting client.
    pub resp: oneshot::Sender<HaResult<LogIndex>>,
}

/// Queue of pending ReadIndex requests on the leader.
pub struct ReadOnlyQueue {
    /// Ordered list of pending reads (FIFO).
    pending: Vec<ReadRequest>,
    /// Next request ID.
    next_id: u64,
    mode: ReadMode,
}

impl ReadOnlyQueue {
    pub fn new(mode: ReadMode) -> Self {
        Self { pending: Vec::new(), next_id: 1, mode }
    }

    /// Add a new read request at the current commit index.
    pub fn add(&mut self, index: LogIndex, resp: oneshot::Sender<HaResult<LogIndex>>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.push(ReadRequest { id, index, acks: HashSet::new(), resp });
        id
    }

    /// Record a heartbeat acknowledgement from `peer`.
    /// Returns list of requests that now have quorum (to be resolved).
    pub fn ack(&mut self, peer: NodeId, quorum: usize) -> Vec<ReadRequest> {
        for req in &mut self.pending {
            req.acks.insert(peer);
        }
        self.drain_ready(quorum)
    }

    /// Drain requests that have accumulated enough acks.
    fn drain_ready(&mut self, quorum: usize) -> Vec<ReadRequest> {
        let mut ready = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].acks.len() >= quorum {
                ready.push(self.pending.remove(i));
            } else {
                i += 1;
            }
        }
        ready
    }

    /// Check if LeaseRead is safe given last heartbeat time.
    pub fn lease_valid(&self, last_heartbeat: std::time::Instant, timeout: std::time::Duration) -> bool {
        self.mode == ReadMode::LeaseRead
            && last_heartbeat.elapsed() < timeout
    }

    /// Drain all requests (on leader step-down).
    pub fn drain_all(&mut self) -> Vec<ReadRequest> {
        std::mem::take(&mut self.pending)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn mode(&self) -> ReadMode { self.mode }
}

/// Lease tracker for LeaseRead.
pub struct LeaderLease {
    granted_at: Option<std::time::Instant>,
    duration: std::time::Duration,
}

impl LeaderLease {
    pub fn new(duration: std::time::Duration) -> Self {
        Self { granted_at: None, duration }
    }

    pub fn renew(&mut self) {
        self.granted_at = Some(std::time::Instant::now());
    }

    pub fn is_valid(&self) -> bool {
        self.granted_at
            .map(|t| t.elapsed() < self.duration)
            .unwrap_or(false)
    }

    pub fn invalidate(&mut self) {
        self.granted_at = None;
    }
}

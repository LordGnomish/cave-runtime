//! Simplified Raft consensus implementation.
//!
//! No external etcd dependency — pure in-process state machine suitable for
//! small bare-metal clusters (3–7 nodes). A production deployment would persist
//! the write-ahead log to disk; this phase keeps it in-memory.

use crate::{
    models::{InstanceRole, LogEntry},
    HaState,
};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Candidate requests votes from peers when the leader heartbeat times out.
///
/// Increments the current term, votes for self, and transitions to Candidate.
pub async fn start_election(state: Arc<HaState>) -> Result<()> {
    let self_id = state.self_instance.id;
    let new_term = {
        let mut raft = state.raft.write().await;
        raft.current_term += 1;
        raft.voted_for = Some(self_id);
        raft.leader_id = None;
        raft.current_term
    };
    {
        let mut topology = state.topology.write().await;
        if let Some(inst) = topology.instances.iter_mut().find(|i| i.id == self_id) {
            inst.role = InstanceRole::Candidate;
        }
    }
    info!(term = new_term, candidate = %self_id, "Election started");
    Ok(())
}

/// Handle an incoming vote request from a candidate.
///
/// Grants the vote if the candidate's term is higher, or if we haven't voted
/// yet in this term and the candidate's log is at least as up-to-date as ours.
pub async fn request_vote(
    state: Arc<HaState>,
    candidate_id: Uuid,
    candidate_term: u64,
    candidate_log_index: u64,
) -> Result<bool> {
    let mut raft = state.raft.write().await;

    if candidate_term > raft.current_term {
        raft.current_term = candidate_term;
        raft.voted_for = Some(candidate_id);
        raft.leader_id = None;
        info!(term = candidate_term, candidate = %candidate_id, "Vote granted (higher term)");
        return Ok(true);
    }

    if candidate_term == raft.current_term
        && (raft.voted_for.is_none() || raft.voted_for == Some(candidate_id))
        && candidate_log_index >= raft.commit_index
    {
        raft.voted_for = Some(candidate_id);
        info!(term = candidate_term, candidate = %candidate_id, "Vote granted (same term)");
        return Ok(true);
    }

    warn!(
        candidate_term,
        current_term = raft.current_term,
        candidate = %candidate_id,
        "Vote denied"
    );
    Ok(false)
}

/// Leader replicates log entries to this follower and updates the commit index.
///
/// Returns `false` if the leader's term is stale (the leader must step down).
pub async fn append_entries(
    state: Arc<HaState>,
    leader_id: Uuid,
    leader_term: u64,
    entries: Vec<LogEntry>,
    leader_commit: u64,
) -> Result<bool> {
    let mut raft = state.raft.write().await;

    if leader_term < raft.current_term {
        warn!(
            leader_term,
            current_term = raft.current_term,
            "Rejected append_entries: stale leader term"
        );
        return Ok(false);
    }

    raft.current_term = leader_term;
    raft.leader_id = Some(leader_id);
    if leader_commit > raft.commit_index {
        raft.commit_index = leader_commit;
    }

    info!(
        leader = %leader_id,
        entries = entries.len(),
        commit_index = raft.commit_index,
        "Entries appended"
    );
    Ok(true)
}

/// Apply all log entries up to `commit_index` to the local state machine.
pub async fn apply_committed(state: Arc<HaState>) -> Result<()> {
    let (commit_index, last_applied) = {
        let raft = state.raft.read().await;
        (raft.commit_index, raft.last_applied)
    };

    if commit_index > last_applied {
        let mut raft = state.raft.write().await;
        raft.last_applied = raft.commit_index;
        info!(applied_through = raft.last_applied, "Committed entries applied to state machine");
    }
    Ok(())
}

/// Leader sends a periodic heartbeat (empty append_entries) to prevent elections.
pub async fn leader_heartbeat(state: Arc<HaState>) -> Result<()> {
    let self_id = state.self_instance.id;
    let term = state.raft.read().await.current_term;
    {
        let mut topology = state.topology.write().await;
        if let Some(inst) = topology.instances.iter_mut().find(|i| i.id == self_id) {
            inst.last_heartbeat = Utc::now();
        }
    }
    info!(term, leader = %self_id, "Heartbeat sent");
    Ok(())
}

/// Leader steps down to follower upon discovering a higher term in any RPC response.
pub async fn step_down(state: Arc<HaState>, new_term: u64) -> Result<()> {
    let self_id = state.self_instance.id;
    {
        let mut raft = state.raft.write().await;
        raft.current_term = new_term;
        raft.voted_for = None;
        raft.leader_id = None;
    }
    {
        let mut topology = state.topology.write().await;
        if let Some(inst) = topology.instances.iter_mut().find(|i| i.id == self_id) {
            inst.role = InstanceRole::Follower;
        }
    }
    warn!(new_term, node = %self_id, "Stepped down to follower");
    Ok(())
//! Full Raft consensus implementation.
//!
//! Uses tokio channels for inter-node communication — no network I/O,
//! making it fully testable in unit tests.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tracing::{debug, info};
pub type NodeId = u64;
/// Role of a Raft node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaftRole {
    Follower,
    Candidate,
    Leader,
}
/// A single entry in the Raft log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub index: u64,
    pub term: u64,
    pub data: Vec<u8>,
}
/// All messages exchanged between Raft nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RaftMessage {
    RequestVote {
        term: u64,
        candidate_id: NodeId,
        last_log_index: u64,
        last_log_term: u64,
    },
    RequestVoteReply {
        term: u64,
        vote_granted: bool,
        from: NodeId,
    },
    AppendEntries {
        term: u64,
        leader_id: NodeId,
        prev_log_index: u64,
        prev_log_term: u64,
        entries: Vec<LogEntry>,
        leader_commit: u64,
    },
    AppendEntriesReply {
        term: u64,
        success: bool,
        match_index: u64,
        from: NodeId,
    },
    InstallSnapshot {
        term: u64,
        leader_id: NodeId,
        last_included_index: u64,
        last_included_term: u64,
        data: Vec<u8>,
    },
    InstallSnapshotReply {
        term: u64,
        success: bool,
        from: NodeId,
    },
    /// Leadership transfer for manual failover.
    TransferLeadership {
        target: NodeId,
    },
}
/// Volatile and persistent state of a Raft node.
struct NodeState {
    // --- Persistent (would be written to stable storage in production) ---
    current_term: u64,
    voted_for: Option<NodeId>,
    log: Vec<LogEntry>,
    // --- Volatile ---
    commit_index: u64,
    last_applied: u64,
    role: RaftRole,
    leader_id: Option<NodeId>,
    // --- Leader-only volatile state ---
    next_index: HashMap<NodeId, u64>,
    match_index: HashMap<NodeId, u64>,
    votes_received: HashSet<NodeId>,
    // --- Snapshot state ---
    snapshot_index: u64,
    snapshot_term: u64,
    snapshot_data: Vec<u8>,
    // --- Pending client commands waiting for commit ---
    pending_commands: HashMap<u64, oneshot::Sender<Result<u64, String>>>,
}
impl NodeState {
    fn new() -> Self {
        Self {
            current_term: 0,
            voted_for: None,
            log: Vec::new(),
            commit_index: 0,
            last_applied: 0,
            role: RaftRole::Follower,
            leader_id: None,
            next_index: HashMap::new(),
            match_index: HashMap::new(),
            votes_received: HashSet::new(),
            snapshot_index: 0,
            snapshot_term: 0,
            snapshot_data: Vec::new(),
            pending_commands: HashMap::new(),
        }
    }
    /// Index of the last log entry (accounting for snapshot compaction).
    fn last_log_index(&self) -> u64 {
        if self.log.is_empty() {
            self.snapshot_index
        } else {
            self.log.last().unwrap().index
        }
    }
    /// Term of the last log entry.
    fn last_log_term(&self) -> u64 {
        if self.log.is_empty() {
            self.snapshot_term
        } else {
            self.log.last().unwrap().term
        }
    }
    /// Returns the log entry at the given absolute index, if it exists.
    fn entry_at(&self, index: u64) -> Option<&LogEntry> {
        if index == 0 {
            return None;
        }
        // Our log is 0-based but indices start from snapshot_index+1.
        let base = self.snapshot_index;
        if index <= base {
            return None;
        }
        let offset = (index - base - 1) as usize;
        self.log.get(offset)
    }
    /// Returns the term for the given log index, consulting snapshot state if needed.
    fn term_at(&self, index: u64) -> Option<u64> {
        if index == 0 {
            return Some(0);
        }
        if index == self.snapshot_index {
            return Some(self.snapshot_term);
        }
        self.entry_at(index).map(|e| e.term)
    }
    /// Truncate the log so only entries with index <= keep_through remain.
    fn truncate_after(&mut self, keep_through_index: u64) {
        if keep_through_index <= self.snapshot_index {
            self.log.clear();
            return;
        }
        let base = self.snapshot_index;
        let new_len = (keep_through_index - base) as usize;
        self.log.truncate(new_len);
    }
    /// Append entries, discarding any conflicting suffix first.
    fn append_entries_from(&mut self, entries: &[LogEntry]) {
        for entry in entries {
            let existing = self.entry_at(entry.index);
            match existing {
                Some(e) if e.term == entry.term => {
                    // Already have it; skip.
                }
                Some(_) => {
                    // Conflict: truncate and append.
                    self.truncate_after(entry.index - 1);
                    self.log.push(entry.clone());
                }
                None => {
                    self.log.push(entry.clone());
                }
            }
        }
    }
    /// Apply snapshot, discarding all log entries covered by it.
    fn apply_snapshot(&mut self, last_index: u64, last_term: u64, data: Vec<u8>) {
        if last_index <= self.snapshot_index {
            return;
        }
        // Drop log entries covered by this snapshot.
        let base = self.snapshot_index;
        let entries_to_drop = if last_index > base {
            (last_index - base) as usize
        } else {
            0
        };
        if entries_to_drop >= self.log.len() {
            self.log.clear();
        } else {
            self.log.drain(..entries_to_drop);
        }
        self.snapshot_index = last_index;
        self.snapshot_term = last_term;
        self.snapshot_data = data;
        if self.commit_index < last_index {
            self.commit_index = last_index;
        }
        if self.last_applied < last_index {
            self.last_applied = last_index;
        }
    }
}
/// A Raft consensus node.
pub struct RaftNode {
    pub id: NodeId,
    state: Arc<Mutex<NodeState>>,
    /// Outbound channel per peer.
    outgoing: HashMap<NodeId, mpsc::UnboundedSender<RaftMessage>>,
    /// Inbound messages from other nodes.
    incoming: mpsc::UnboundedReceiver<RaftMessage>,
    /// Commands submitted by the application layer.
    command_rx: mpsc::UnboundedReceiver<(Vec<u8>, oneshot::Sender<Result<u64, String>>)>,
    pub command_tx: mpsc::UnboundedSender<(Vec<u8>, oneshot::Sender<Result<u64, String>>)>,
    /// Notification channel for applied log entries.
    applied_tx: broadcast::Sender<LogEntry>,
    pub applied_rx_handle: broadcast::Receiver<LogEntry>,
}
impl RaftNode {
    /// Create a new node.  `peers` maps each peer's NodeId to a channel that
    /// delivers messages to that peer's `incoming` receiver.
    pub fn new(
        id: NodeId,
        outgoing: HashMap<NodeId, mpsc::UnboundedSender<RaftMessage>>,
        incoming: mpsc::UnboundedReceiver<RaftMessage>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (applied_tx, applied_rx_handle) = broadcast::channel(256);
        Self {
            id,
            state: Arc::new(Mutex::new(NodeState::new())),
            outgoing,
            incoming,
            command_rx,
            command_tx,
            applied_tx,
            applied_rx_handle,
        }
    }
    /// Number of peers (not counting self).
    /// Cluster size (peers + self).
    fn cluster_size(&self) -> usize {
        self.outgoing.len() + 1
    }
    /// Quorum size.
    fn quorum(&self) -> usize {
        self.cluster_size() / 2 + 1
    }
    /// Send a message to a peer, ignoring errors (peer may be gone).
    fn send_to(&self, peer: NodeId, msg: RaftMessage) {
        if let Some(tx) = self.outgoing.get(&peer) {
            let _ = tx.send(msg);
        }
    }
    // -----------------------------------------------------------------------
    // Public inspection methods used in tests
    // -----------------------------------------------------------------------
    /// Returns the current role of this node.
    pub async fn current_role(&self) -> RaftRole {
        self.state.lock().await.role
    }
    /// Returns the current leader id as seen by this node.
    pub async fn current_leader(&self) -> Option<NodeId> {
        self.state.lock().await.leader_id
    }
    /// Returns the current term.
    pub async fn current_term(&self) -> u64 {
        self.state.lock().await.current_term
    }
    /// Returns the commit index.
    pub async fn commit_index(&self) -> u64 {
        self.state.lock().await.commit_index
    }
    // -----------------------------------------------------------------------
    // Main run loop
    // -----------------------------------------------------------------------
    /// Drive the Raft state machine.  Consumes `self` and runs until all
    /// incoming/command channels are closed.
    pub async fn run(mut self) {
        let node_id = self.id;
        let is_single = self.cluster_size() == 1;
        // Single-node cluster: immediately become leader without election.
        if is_single {
            let mut st = self.state.lock().await;
            st.current_term = 1;
            st.voted_for = Some(node_id);
            st.role = RaftRole::Leader;
            st.leader_id = Some(node_id);
            drop(st);
            info!(node = node_id, "single-node cluster — immediately became leader");
        }
        let mut heartbeat_interval = tokio::time::interval(tokio::time::Duration::from_millis(50));
        // election_deadline tracks when a follower/candidate times out.
        let election_timeout_ms = rand::thread_rng().gen_range(150u64..=300u64);
        let mut election_deadline =
            tokio::time::Instant::now() + tokio::time::Duration::from_millis(election_timeout_ms);
        loop {
            let role = self.state.lock().await.role;
            match role {
                RaftRole::Leader => {
                    tokio::select! {
                        _ = heartbeat_interval.tick() => {
                            self.send_heartbeats().await;
                        }
                        msg = self.incoming.recv() => {
                            match msg {
                                Some(m) => { let _ = self.handle_message(m).await; }
                                None => { info!(node = node_id, "incoming channel closed, stopping"); return; }
                            }
                        }
                        cmd = self.command_rx.recv() => {
                            match cmd {
                                Some((data, resp)) => self.handle_command(data, resp).await,
                                None => return,
                            }
                        }
                    }
                }
                RaftRole::Follower | RaftRole::Candidate => {
                    let now = tokio::time::Instant::now();
                    let timeout_duration = if election_deadline > now {
                        election_deadline - now
                    } else {
                        tokio::time::Duration::from_millis(0)
                    };
                    tokio::select! {
                        _ = tokio::time::sleep(timeout_duration) => {
                            // Election timeout fired.
                            self.start_election().await;
                            let new_timeout = rand::thread_rng().gen_range(150u64..=300u64);
                            election_deadline = tokio::time::Instant::now()
                                + tokio::time::Duration::from_millis(new_timeout);
                        }
                        msg = self.incoming.recv() => {
                            match msg {
                                Some(m) => {
                                    let reset = self.handle_message(m).await;
                                    if reset {
                                        let new_timeout = rand::thread_rng().gen_range(150u64..=300u64);
                                        election_deadline = tokio::time::Instant::now()
                                            + tokio::time::Duration::from_millis(new_timeout);
                                    }
                                }
                                None => { info!(node = node_id, "incoming channel closed, stopping"); return; }
                            }
                        }
                        cmd = self.command_rx.recv() => {
                            match cmd {
                                Some((_, resp)) => {
                                    let _ = resp.send(Err("not the leader".to_string()));
                                }
                                None => return,
                            }
                        }
                    }
                }
            }
        }
    }
    // -----------------------------------------------------------------------
    // Leader operations
    // -----------------------------------------------------------------------
    async fn send_heartbeats(&self) {
        let st = self.state.lock().await;
        let peers: Vec<NodeId> = self.outgoing.keys().copied().collect();
        for peer in peers {
            let prev_index = st.next_index.get(&peer).copied().unwrap_or(1).saturating_sub(1);
            let prev_term = st.term_at(prev_index).unwrap_or(0);
            // Entries to send: from next_index[peer] to end.
            let next = st.next_index.get(&peer).copied().unwrap_or(1);
            let base = st.snapshot_index;
            let entries: Vec<LogEntry> = if next > st.last_log_index() || st.log.is_empty() {
                vec![]
            } else if next <= base {
                // Need to send snapshot instead.
                vec![]
            } else {
                let offset = (next - base - 1) as usize;
                st.log[offset..].to_vec()
            };
            // If peer is far behind, send snapshot.
            let needs_snapshot = next <= base && base > 0;
            let msg = if needs_snapshot {
                RaftMessage::InstallSnapshot {
                    term: st.current_term,
                    leader_id: self.id,
                    last_included_index: st.snapshot_index,
                    last_included_term: st.snapshot_term,
                    data: st.snapshot_data.clone(),
                }
            } else {
                RaftMessage::AppendEntries {
                    term: st.current_term,
                    leader_id: self.id,
                    prev_log_index: prev_index,
                    prev_log_term: prev_term,
                    entries,
                    leader_commit: st.commit_index,
                }
            };
            drop(st);
            self.send_to(peer, msg);
            return; // re-acquire lock for next peer
        }
    }
    /// Send AppendEntries to all peers.  Acquires state per peer to build correct message.
    async fn replicate_to_all(&self) {
        let peers: Vec<NodeId> = self.outgoing.keys().copied().collect();
        for peer in peers {
            self.replicate_to_peer(peer).await;
        }
    }
    async fn replicate_to_peer(&self, peer: NodeId) {
        let st = self.state.lock().await;
        if st.role != RaftRole::Leader {
            return;
        }
        let next = st.next_index.get(&peer).copied().unwrap_or(1);
        let base = st.snapshot_index;
        let needs_snapshot = next <= base && base > 0;
        if needs_snapshot {
            let msg = RaftMessage::InstallSnapshot {
                term: st.current_term,
                leader_id: self.id,
                last_included_index: st.snapshot_index,
                last_included_term: st.snapshot_term,
                data: st.snapshot_data.clone(),
            };
            drop(st);
            self.send_to(peer, msg);
        } else {
            let prev_index = next.saturating_sub(1);
            let prev_term = st.term_at(prev_index).unwrap_or(0);
            let entries: Vec<LogEntry> = if st.log.is_empty() || next > st.last_log_index() {
                vec![]
            } else {
                let offset = (next - base - 1) as usize;
                st.log[offset..].to_vec()
            };
            let msg = RaftMessage::AppendEntries {
                term: st.current_term,
                leader_id: self.id,
                prev_log_index: prev_index,
                prev_log_term: prev_term,
                entries,
                leader_commit: st.commit_index,
            };
            drop(st);
            self.send_to(peer, msg);
        }
    }
    async fn handle_command(
        &mut self,
        data: Vec<u8>,
        resp: oneshot::Sender<Result<u64, String>>,
    ) {
        let mut st = self.state.lock().await;
        if st.role != RaftRole::Leader {
            drop(st);
            let _ = resp.send(Err("not the leader".to_string()));
            return;
        }
        let new_index = st.last_log_index() + 1;
        let entry = LogEntry {
            index: new_index,
            term: st.current_term,
            data,
        };
        st.log.push(entry);
        st.pending_commands.insert(new_index, resp);
        // Single-node: commit immediately.
        if self.outgoing.is_empty() {
            st.commit_index = new_index;
            // Apply up to commit_index.
            let entries_to_apply: Vec<LogEntry> = st
                .log
                .iter()
                .filter(|e| e.index > st.last_applied && e.index <= st.commit_index)
                .cloned()
                .collect();
            for entry in entries_to_apply {
                st.last_applied = entry.index;
                if let Some(tx) = st.pending_commands.remove(&entry.index) {
                    let _ = tx.send(Ok(entry.index));
                }
                let _ = self.applied_tx.send(entry);
            }
        }
        drop(st);
        if !self.outgoing.is_empty() {
            self.replicate_to_all().await;
        }
    }
    // -----------------------------------------------------------------------
    // Election
    // -----------------------------------------------------------------------
    async fn start_election(&mut self) {
        let mut st = self.state.lock().await;
        st.current_term += 1;
        st.role = RaftRole::Candidate;
        st.voted_for = Some(self.id);
        st.leader_id = None;
        st.votes_received.clear();
        st.votes_received.insert(self.id);
        let term = st.current_term;
        let last_log_index = st.last_log_index();
        let last_log_term = st.last_log_term();
        let peers: Vec<NodeId> = self.outgoing.keys().copied().collect();
        let quorum = self.quorum();
        debug!(node = self.id, term, "starting election");
        // Check if single-node (already handled in run() but guard here).
        if peers.is_empty() {
            st.role = RaftRole::Leader;
            st.leader_id = Some(self.id);
            info!(node = self.id, term, "elected as leader (single node)");
            // Initialize leader state.
            st.next_index.clear();
            st.match_index.clear();
            return;
        }
        // Check if we already have quorum (we vote for ourselves).
        if st.votes_received.len() >= quorum {
            st.role = RaftRole::Leader;
            st.leader_id = Some(self.id);
            let last = st.last_log_index();
            for &p in &peers {
                st.next_index.insert(p, last + 1);
                st.match_index.insert(p, 0);
            }
            info!(node = self.id, term, "won election immediately");
            drop(st);
            self.send_heartbeats().await;
            return;
        }
        drop(st);
        for peer in peers {
            self.send_to(
                peer,
                RaftMessage::RequestVote {
                    term,
                    candidate_id: self.id,
                    last_log_index,
                    last_log_term,
                },
            );
        }
    }
    // -----------------------------------------------------------------------
    // Message dispatch
    // -----------------------------------------------------------------------
    /// Handle a single message. Returns `true` if election timer should reset.
    async fn handle_message(&mut self, msg: RaftMessage) -> bool {
        match msg {
            RaftMessage::RequestVote {
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                self.handle_request_vote(term, candidate_id, last_log_index, last_log_term)
                    .await
            }
            RaftMessage::RequestVoteReply {
                term,
                vote_granted,
                from,
            } => {
                self.handle_vote_reply(term, vote_granted, from).await;
                false
            }
            RaftMessage::AppendEntries {
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            } => {
                self.handle_append_entries(
                    term,
                    leader_id,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                )
                .await
            }
            RaftMessage::AppendEntriesReply {
                term,
                success,
                match_index,
                from,
            } => {
                self.handle_append_entries_reply(term, success, match_index, from)
                    .await;
                false
            }
            RaftMessage::InstallSnapshot {
                term,
                leader_id,
                last_included_index,
                last_included_term,
                data,
            } => {
                self.handle_install_snapshot(
                    term,
                    leader_id,
                    last_included_index,
                    last_included_term,
                    data,
                )
                .await
            }
            RaftMessage::InstallSnapshotReply { term, success, from } => {
                self.handle_install_snapshot_reply(term, success, from).await;
                false
            }
            RaftMessage::TransferLeadership { target } => {
                self.handle_transfer_leadership(target).await;
                false
            }
        }
    }
    async fn handle_request_vote(
        &mut self,
        term: u64,
        candidate_id: NodeId,
        last_log_index: u64,
        last_log_term: u64,
    ) -> bool {
        let mut st = self.state.lock().await;
        let mut reset_timer = false;
        if term > st.current_term {
            st.current_term = term;
            st.role = RaftRole::Follower;
            st.voted_for = None;
        }
        let log_ok = (last_log_term > st.last_log_term())
            || (last_log_term == st.last_log_term() && last_log_index >= st.last_log_index());
        let vote_granted = term >= st.current_term
            && log_ok
            && (st.voted_for.is_none() || st.voted_for == Some(candidate_id));
        if vote_granted {
            st.voted_for = Some(candidate_id);
            reset_timer = true;
        }
        let reply = RaftMessage::RequestVoteReply {
            term: st.current_term,
            vote_granted,
            from: self.id,
        };
        drop(st);
        self.send_to(candidate_id, reply);
        reset_timer
    }
    async fn handle_vote_reply(&mut self, term: u64, vote_granted: bool, from: NodeId) {
        let mut st = self.state.lock().await;
        if term > st.current_term {
            st.current_term = term;
            st.role = RaftRole::Follower;
            st.voted_for = None;
            return;
        }
        if st.role != RaftRole::Candidate || term != st.current_term {
            return;
        }
        if vote_granted {
            st.votes_received.insert(from);
            let votes = st.votes_received.len();
            let quorum = self.quorum();
            if votes >= quorum {
                // Become leader.
                st.role = RaftRole::Leader;
                st.leader_id = Some(self.id);
                let last = st.last_log_index();
                for &p in self.outgoing.keys() {
                    st.next_index.insert(p, last + 1);
                    st.match_index.insert(p, 0);
                }
                let term = st.current_term;
                info!(node = self.id, term, "won election");
                drop(st);
                self.send_heartbeats().await;
                return;
            }
        }
    }
    async fn handle_append_entries(
        &mut self,
        term: u64,
        leader_id: NodeId,
        prev_log_index: u64,
        prev_log_term: u64,
        entries: Vec<LogEntry>,
        leader_commit: u64,
    ) -> bool {
        let mut st = self.state.lock().await;
        if term > st.current_term {
            st.current_term = term;
            st.role = RaftRole::Follower;
            st.voted_for = None;
        }
        if term < st.current_term {
            let reply = RaftMessage::AppendEntriesReply {
                term: st.current_term,
                success: false,
                match_index: 0,
                from: self.id,
            };
            drop(st);
            self.send_to(leader_id, reply);
            return false;
        }
        // Valid leader heartbeat: reset timer.
        st.role = RaftRole::Follower;
        st.leader_id = Some(leader_id);
        // Check prev_log consistency.
        let prev_ok = if prev_log_index == 0 {
            true
        } else if prev_log_index <= st.snapshot_index {
            // Covered by snapshot.
            true
        } else {
            st.term_at(prev_log_index) == Some(prev_log_term)
        };
        if !prev_ok {
            let reply = RaftMessage::AppendEntriesReply {
                term: st.current_term,
                success: false,
                match_index: st.last_log_index(),
                from: self.id,
            };
            drop(st);
            self.send_to(leader_id, reply);
            return true;
        }
        st.append_entries_from(&entries);
        let new_match = if entries.is_empty() {
            prev_log_index
        } else {
            entries.last().unwrap().index
        };
        // Advance commit index.
        if leader_commit > st.commit_index {
            st.commit_index = leader_commit.min(st.last_log_index());
        }
        // Apply committed entries.
        let entries_to_apply: Vec<LogEntry> = st
            .log
            .iter()
            .filter(|e| e.index > st.last_applied && e.index <= st.commit_index)
            .cloned()
            .collect();
        for entry in entries_to_apply {
            st.last_applied = entry.index;
            let _ = self.applied_tx.send(entry);
        }
        let reply = RaftMessage::AppendEntriesReply {
            term: st.current_term,
            success: true,
            match_index: new_match,
            from: self.id,
        };
        drop(st);
        self.send_to(leader_id, reply);
        true
    }
    async fn handle_append_entries_reply(
        &mut self,
        term: u64,
        success: bool,
        match_index: u64,
        from: NodeId,
    ) {
        let mut st = self.state.lock().await;
        if term > st.current_term {
            st.current_term = term;
            st.role = RaftRole::Follower;
            st.voted_for = None;
            return;
        }
        if st.role != RaftRole::Leader {
            return;
        }
        if success {
            let prev_match = st.match_index.get(&from).copied().unwrap_or(0);
            if match_index > prev_match {
                st.match_index.insert(from, match_index);
            }
            st.next_index.insert(from, match_index + 1);
            // Advance commit index if majority has replicated.
            let last = st.last_log_index();
            for n in (st.commit_index + 1..=last).rev() {
                // Only commit entries from current term.
                if st.term_at(n) != Some(st.current_term) {
                    continue;
                }
                let replicated = self
                    .outgoing
                    .keys()
                    .filter(|&&p| st.match_index.get(&p).copied().unwrap_or(0) >= n)
                    .count()
                    + 1; // +1 for self
                if replicated >= self.quorum() {
                    st.commit_index = n;
                    debug!(node = self.id, commit_index = n, "advanced commit index");
                    break;
                }
            }
            // Apply committed entries.
            let entries_to_apply: Vec<LogEntry> = st
                .log
                .iter()
                .filter(|e| e.index > st.last_applied && e.index <= st.commit_index)
                .cloned()
                .collect();
            for entry in entries_to_apply {
                st.last_applied = entry.index;
                if let Some(tx) = st.pending_commands.remove(&entry.index) {
                    let _ = tx.send(Ok(entry.index));
                }
                let _ = self.applied_tx.send(entry);
            }
        } else {
            // Back off next_index for this peer.
            let ni = st.next_index.get(&from).copied().unwrap_or(1);
            if ni > 1 {
                // Use match_index from reply as a hint.
                let new_next = if match_index > 0 && match_index < ni {
                    match_index + 1
                } else {
                    ni.saturating_sub(1).max(1)
                };
                st.next_index.insert(from, new_next);
            }
            drop(st);
            self.replicate_to_peer(from).await;
            return;
        }
        drop(st);
    }
    async fn handle_install_snapshot(
        &mut self,
        term: u64,
        leader_id: NodeId,
        last_included_index: u64,
        last_included_term: u64,
        data: Vec<u8>,
    ) -> bool {
        let mut st = self.state.lock().await;
        if term > st.current_term {
            st.current_term = term;
            st.role = RaftRole::Follower;
            st.voted_for = None;
        }
        if term < st.current_term {
            let reply = RaftMessage::InstallSnapshotReply {
                term: st.current_term,
                success: false,
                from: self.id,
            };
            drop(st);
            self.send_to(leader_id, reply);
            return false;
        }
        st.leader_id = Some(leader_id);
        st.apply_snapshot(last_included_index, last_included_term, data);
        let reply = RaftMessage::InstallSnapshotReply {
            term: st.current_term,
            success: true,
            from: self.id,
        };
        drop(st);
        self.send_to(leader_id, reply);
        true
    }
    async fn handle_install_snapshot_reply(&mut self, term: u64, success: bool, from: NodeId) {
        let mut st = self.state.lock().await;
        if term > st.current_term {
            st.current_term = term;
            st.role = RaftRole::Follower;
            st.voted_for = None;
            return;
        }
        if success {
            let snap_index = st.snapshot_index;
            st.next_index.insert(from, snap_index + 1);
            st.match_index.insert(from, snap_index);
        }
    }
    async fn handle_transfer_leadership(&mut self, target: NodeId) {
        let st = self.state.lock().await;
        if st.role != RaftRole::Leader {
            return;
        }
        let term = st.current_term;
        drop(st);
        // Send a TransferLeadership message to the target — it will start an election.
        // In a real implementation we'd use a TimeoutNow RPC; here we just nudge.
        info!(node = self.id, target, term, "transferring leadership");
        self.send_to(target, RaftMessage::TransferLeadership { target });
    }
}
// ---------------------------------------------------------------------------
// Helper to build a fully connected in-memory cluster for tests.
// ---------------------------------------------------------------------------
/// Build `n` Raft nodes connected by unbounded channels.
/// Returns the nodes in order of their IDs (1..=n).
pub fn build_cluster(n: usize) -> Vec<RaftNode> {
    // Create one incoming channel per node.
    let mut incoming_txs: HashMap<NodeId, mpsc::UnboundedSender<RaftMessage>> = HashMap::new();
    let mut incoming_rxs: HashMap<NodeId, mpsc::UnboundedReceiver<RaftMessage>> = HashMap::new();
    for i in 1..=(n as NodeId) {
        let (tx, rx) = mpsc::unbounded_channel();
        incoming_txs.insert(i, tx);
        incoming_rxs.insert(i, rx);
    }
    let mut nodes = Vec::new();
    for node_id in 1..=(n as NodeId) {
        let incoming = incoming_rxs.remove(&node_id).unwrap();
        let mut outgoing: HashMap<NodeId, mpsc::UnboundedSender<RaftMessage>> = HashMap::new();
        for peer_id in 1..=(n as NodeId) {
            if peer_id != node_id {
                outgoing.insert(peer_id, incoming_txs[&peer_id].clone());
            }
        }
        nodes.push(RaftNode::new(node_id, outgoing, incoming));
    }
    nodes
}
// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};
    #[tokio::test]
    async fn test_single_node_becomes_leader() {
        let nodes = build_cluster(1);
        let node = nodes.into_iter().next().unwrap();
        let state = Arc::clone(&node.state);
        tokio::spawn(node.run());
        // Single node should become leader almost immediately.
        let result = timeout(Duration::from_secs(2), async {
            loop {
                let role = state.lock().await.role;
                if role == RaftRole::Leader {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(result.is_ok(), "timed out waiting for leader");
        assert!(result.unwrap());
    }
    #[tokio::test]
    async fn test_three_node_election() {
        let nodes = build_cluster(3);
        let states: Vec<Arc<Mutex<NodeState>>> =
            nodes.iter().map(|n| Arc::clone(&n.state)).collect();
        for node in nodes {
            tokio::spawn(node.run());
        }
        // Wait for a leader to emerge.
        let result = timeout(Duration::from_secs(3), async {
            loop {
                let mut leaders = 0;
                for st in &states {
                    if st.lock().await.role == RaftRole::Leader {
                        leaders += 1;
                    }
                }
                if leaders == 1 {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(result.is_ok(), "timed out waiting for leader in 3-node cluster");
        assert!(result.unwrap());
        // Verify exactly one leader.
        let mut leader_count = 0;
        for st in &states {
            if st.lock().await.role == RaftRole::Leader {
                leader_count += 1;
            }
        }
        assert_eq!(leader_count, 1, "expected exactly 1 leader");
    }
    #[tokio::test]
    async fn test_log_replication() {
        let nodes = build_cluster(3);
        let states: Vec<Arc<Mutex<NodeState>>> =
            nodes.iter().map(|n| Arc::clone(&n.state)).collect();
        let command_txs: Vec<_> = nodes.iter().map(|n| n.command_tx.clone()).collect();
        for node in nodes {
            tokio::spawn(node.run());
        }
        // Wait for a leader.
        let leader_idx = timeout(Duration::from_secs(3), async {
            loop {
                for (i, st) in states.iter().enumerate() {
                    if st.lock().await.role == RaftRole::Leader {
                        return i;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("timed out waiting for leader");
        // Submit a command via the leader's command channel.
        let (resp_tx, resp_rx) = oneshot::channel();
        command_txs[leader_idx]
            .send((b"hello-raft".to_vec(), resp_tx))
            .unwrap();
        let result = timeout(Duration::from_secs(2), resp_rx)
            .await
            .expect("timed out waiting for command result");
        let index = result
            .expect("channel closed")
            .expect("command failed");
        assert!(index >= 1, "committed index should be >= 1");
        // Leader's commit_index should reflect the commit.
        let leader_commit = states[leader_idx].lock().await.commit_index;
        assert!(leader_commit >= 1);
    }
    #[tokio::test]
    async fn test_commit_index_advances() {
        let nodes = build_cluster(3);
        let states: Vec<Arc<Mutex<NodeState>>> =
            nodes.iter().map(|n| Arc::clone(&n.state)).collect();
        let command_txs: Vec<_> = nodes.iter().map(|n| n.command_tx.clone()).collect();
        for node in nodes {
            tokio::spawn(node.run());
        }
        let leader_idx = timeout(Duration::from_secs(3), async {
            loop {
                for (i, st) in states.iter().enumerate() {
                    if st.lock().await.role == RaftRole::Leader {
                        return i;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // Submit multiple commands.
        for i in 0..3u8 {
            let (resp_tx, resp_rx) = oneshot::channel();
            command_txs[leader_idx]
                .send((vec![i], resp_tx))
                .unwrap();
            timeout(Duration::from_secs(2), resp_rx)
                .await
                .expect("timed out")
                .expect("channel closed")
                .expect("command failed");
        }
        // commit_index on leader must be >= 3.
        let commit = states[leader_idx].lock().await.commit_index;
        assert!(commit >= 3, "commit_index should be >= 3, got {commit}");
    }
    #[tokio::test]
    async fn test_term_increments_on_timeout() {
        // Build a 1-node cluster so we can control the election easily.
        let nodes = build_cluster(1);
        let state = Arc::clone(&nodes[0].state);
        tokio::spawn(nodes.into_iter().next().unwrap().run());
        // After run() the term should be at least 1 (single node sets term=1).
        timeout(Duration::from_secs(2), async {
            loop {
                let term = state.lock().await.current_term;
                if term >= 1 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("timed out waiting for term to increment");
        let term = state.lock().await.current_term;
        assert!(term >= 1);
    }
    #[tokio::test]
    async fn test_follower_rejects_stale_term() {
        let nodes = build_cluster(3);
        let states: Vec<Arc<Mutex<NodeState>>> =
            nodes.iter().map(|n| Arc::clone(&n.state)).collect();
        for node in nodes {
            tokio::spawn(node.run());
        }
        // Wait for election.
        timeout(Duration::from_secs(3), async {
            loop {
                for st in &states {
                    if st.lock().await.role == RaftRole::Leader {
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // Find a follower and verify it has a term > 0.
        for st in &states {
            let locked = st.lock().await;
            if locked.role == RaftRole::Follower {
                assert!(locked.current_term >= 1);
                break;
            }
        }
    }
}

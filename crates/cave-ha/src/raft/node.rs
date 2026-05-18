// SPDX-License-Identifier: AGPL-3.0-or-later
//! Core Raft state machine — drives election, replication, snapshotting, and read-only queries.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::config::NodeConfig;
use crate::error::{HaError, HaResult};
use crate::metrics::Metrics;
use crate::raft::log::{LogEntry, MemLog};
use crate::raft::messages::{
    AppendEntries, AppendEntriesReply, InstallSnapshot, InstallSnapshotReply,
    RaftMessage, ReadIndexReply, ReadIndexRequest, RequestVote, RequestVoteReply, TimeoutNow,
};
use crate::raft::read_only::{LeaderLease, ReadMode, ReadOnlyQueue};
use crate::raft::snapshot::{Snapshot, SnapshotReceiver};
use crate::raft::state_machine::StateMachine;
use crate::raft::types::{
    EntryType, LogIndex, MembershipConfig, NodeId, NodeInfo, NodeStatus, Role,
    SnapshotMeta, Term,
};
use crate::transport::Transport;

/// Commands sent to the node actor from external callers.
pub enum NodeCmd {
    Propose { data: Vec<u8>, resp: oneshot::Sender<HaResult<LogIndex>> },
    ReadIndex { resp: oneshot::Sender<HaResult<LogIndex>> },
    AddNode { node: NodeInfo, resp: oneshot::Sender<HaResult<()>> },
    RemoveNode { id: NodeId, resp: oneshot::Sender<HaResult<()>> },
    TransferLeadership { to: NodeId, resp: oneshot::Sender<HaResult<()>> },
    TriggerSnapshot { resp: oneshot::Sender<HaResult<SnapshotMeta>> },
    Status { resp: oneshot::Sender<NodeStatus> },
    Shutdown,
}

/// Public handle to a running Raft node.
#[derive(Clone)]
pub struct RaftHandle {
    pub node_id: NodeId,
    cmd_tx: mpsc::UnboundedSender<NodeCmd>,
    pub msg_tx: mpsc::UnboundedSender<(NodeId, RaftMessage)>,
}

impl RaftHandle {
    pub async fn propose(&self, data: Vec<u8>) -> HaResult<LogIndex> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCmd::Propose { data, resp: tx })?;
        rx.await?
    }

    pub async fn read_index(&self) -> HaResult<LogIndex> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCmd::ReadIndex { resp: tx })?;
        rx.await?
    }

    pub async fn add_node(&self, node: NodeInfo) -> HaResult<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCmd::AddNode { node, resp: tx })?;
        rx.await?
    }

    pub async fn remove_node(&self, id: NodeId) -> HaResult<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCmd::RemoveNode { id, resp: tx })?;
        rx.await?
    }

    pub async fn transfer_leadership(&self, to: NodeId) -> HaResult<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCmd::TransferLeadership { to, resp: tx })?;
        rx.await?
    }

    pub async fn trigger_snapshot(&self) -> HaResult<SnapshotMeta> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCmd::TriggerSnapshot { resp: tx })?;
        rx.await?
    }

    pub async fn status(&self) -> HaResult<NodeStatus> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCmd::Status { resp: tx })?;
        Ok(rx.await?)
    }

    pub async fn shutdown(&self) {
        let _ = self.cmd_tx.send(NodeCmd::Shutdown);
    }

    /// Deliver an incoming Raft message from another node.
    pub fn send_msg(&self, from: NodeId, msg: RaftMessage) -> HaResult<()> {
        self.msg_tx.send((from, msg)).map_err(|_| HaError::Shutdown)
    }
}

/// Per-peer replication state (leader only).
struct PeerState {
    /// Next log index to send to this peer.
    next_index: LogIndex,
    /// Highest log index known to be replicated on this peer.
    match_index: LogIndex,
    /// In-flight AppendEntries requests (pipelining).
    inflight: u64,
    /// Pipeline sequence counter.
    next_seq: u64,
    /// Last time we received a response from this peer.
    last_response: Instant,
    /// Currently installing a snapshot.
    snapshot_in_progress: bool,
    /// Received ack from this peer in current check-quorum window.
    recent_active: bool,
}

impl PeerState {
    fn new(next_index: LogIndex) -> Self {
        Self {
            next_index,
            match_index: 0,
            inflight: 0,
            next_seq: 1,
            last_response: Instant::now(),
            snapshot_in_progress: false,
            recent_active: false,
        }
    }
}

/// The core Raft node actor. Runs in a dedicated tokio task.
pub struct RaftNode {
    // ── Identity ──────────────────────────────────────────────────────────
    id: NodeId,
    config: NodeConfig,

    // ── Persistent state (must survive restart) ───────────────────────────
    current_term: Term,
    voted_for: Option<NodeId>,

    // ── Role ──────────────────────────────────────────────────────────────
    role: Role,
    leader_id: Option<NodeId>,

    // ── Log ───────────────────────────────────────────────────────────────
    log: MemLog,

    // ── Commit / apply ────────────────────────────────────────────────────
    commit_index: LogIndex,
    last_applied: LogIndex,

    // ── Membership ────────────────────────────────────────────────────────
    membership: MembershipConfig,

    // ── Leader state ──────────────────────────────────────────────────────
    peers: HashMap<NodeId, PeerState>,
    // Pending proposals: log index → response channel.
    pending_proposals: HashMap<LogIndex, oneshot::Sender<HaResult<LogIndex>>>,

    // ── Election ──────────────────────────────────────────────────────────
    election_elapsed: u64,
    election_timeout: u64,
    heartbeat_elapsed: u64,
    votes_received: BTreeSet<NodeId>,
    votes_rejected: BTreeSet<NodeId>,
    // Pre-vote tracking.
    pre_votes_received: BTreeSet<NodeId>,

    // ── Check quorum ──────────────────────────────────────────────────────
    check_quorum_elapsed: u64,

    // ── Read-only ─────────────────────────────────────────────────────────
    read_only: ReadOnlyQueue,
    lease: LeaderLease,

    // ── Snapshot ──────────────────────────────────────────────────────────
    snapshot_receiver: Option<SnapshotReceiver>,
    pending_snapshot_resp: Option<oneshot::Sender<HaResult<SnapshotMeta>>>,

    // ── Leadership transfer ───────────────────────────────────────────────
    transfer_to: Option<NodeId>,
    transfer_elapsed: u64,

    // ── Channels ──────────────────────────────────────────────────────────
    msg_rx: mpsc::UnboundedReceiver<(NodeId, RaftMessage)>,
    cmd_rx: mpsc::UnboundedReceiver<NodeCmd>,

    // ── External dependencies ─────────────────────────────────────────────
    transport: Arc<dyn Transport>,
    state_machine: Arc<dyn StateMachine>,
    metrics: Arc<Metrics>,
}

impl RaftNode {
    /// Create a new node and return the public handle.
    pub fn spawn(
        config: NodeConfig,
        initial_members: Vec<NodeInfo>,
        transport: Arc<dyn Transport>,
        state_machine: Arc<dyn StateMachine>,
        metrics: Arc<Metrics>,
    ) -> RaftHandle {
        let (msg_tx, msg_rx) = mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let id = config.id;
        let election_timeout = randomized_timeout(
            config.election_timeout_min,
            config.election_timeout_max,
        );
        let lease_duration = std::time::Duration::from_millis(
            (config.election_timeout_min as f64
                * config.tick_duration.as_millis() as f64
                * config.lease_fraction) as u64,
        );

        let mut voters = BTreeSet::new();
        let mut learners = BTreeSet::new();
        for n in &initial_members {
            if n.is_learner { learners.insert(n.id); } else { voters.insert(n.id); }
        }
        let membership = MembershipConfig { voters, learners, ..Default::default() };

        let node = Self {
            id,
            config: config.clone(),
            current_term: 0,
            voted_for: None,
            role: Role::Follower,
            leader_id: None,
            log: MemLog::new(),
            commit_index: 0,
            last_applied: 0,
            membership,
            peers: HashMap::new(),
            pending_proposals: HashMap::new(),
            election_elapsed: 0,
            election_timeout,
            heartbeat_elapsed: 0,
            votes_received: BTreeSet::new(),
            votes_rejected: BTreeSet::new(),
            pre_votes_received: BTreeSet::new(),
            check_quorum_elapsed: 0,
            read_only: ReadOnlyQueue::new(ReadMode::ReadIndex),
            lease: LeaderLease::new(lease_duration),
            snapshot_receiver: None,
            pending_snapshot_resp: None,
            transfer_to: None,
            transfer_elapsed: 0,
            msg_rx,
            cmd_rx,
            transport,
            state_machine,
            metrics,
        };

        let handle = RaftHandle {
            node_id: id,
            cmd_tx,
            msg_tx,
        };

        tokio::spawn(async move { node.run().await });
        handle
    }

    async fn run(mut self) {
        let mut tick_interval =
            tokio::time::interval(self.config.tick_duration);
        tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // If single-node, self-elect immediately.
        if self.membership.voters.len() == 1
            && self.membership.voters.contains(&self.id)
        {
            self.campaign(false).await;
        }

        loop {
            tokio::select! {
                _ = tick_interval.tick() => {
                    self.tick().await;
                }
                Some((from, msg)) = self.msg_rx.recv() => {
                    self.step(from, msg).await;
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    if self.handle_cmd(cmd).await {
                        break; // Shutdown.
                    }
                }
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Tick — drives timeouts
    // ═══════════════════════════════════════════════════════════════════════

    async fn tick(&mut self) {
        self.election_elapsed += 1;
        self.heartbeat_elapsed += 1;

        match self.role {
            Role::Follower | Role::PreCandidate | Role::Candidate => {
                if self.election_elapsed >= self.election_timeout {
                    self.election_elapsed = 0;
                    if self.config.pre_vote {
                        self.campaign(true).await; // pre-vote phase
                    } else {
                        self.campaign(false).await;
                    }
                }
            }
            Role::Leader => {
                // Heartbeat.
                if self.heartbeat_elapsed >= self.config.heartbeat_interval {
                    self.heartbeat_elapsed = 0;
                    self.broadcast_heartbeat().await;
                }
                // Check quorum.
                if self.config.check_quorum {
                    self.check_quorum_elapsed += 1;
                    if self.check_quorum_elapsed >= self.config.check_quorum_interval {
                        self.check_quorum_elapsed = 0;
                        self.do_check_quorum().await;
                    }
                }
                // Leadership transfer timeout.
                if self.transfer_to.is_some() {
                    self.transfer_elapsed += 1;
                    if self.transfer_elapsed >= self.config.leadership_transfer_timeout {
                        warn!(id = self.id, "leadership transfer timed out, aborting");
                        self.transfer_to = None;
                        self.transfer_elapsed = 0;
                    }
                }
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Campaign — start election (or pre-vote)
    // ═══════════════════════════════════════════════════════════════════════

    /// Initiate a pre-vote round (does not increment term).
    async fn campaign_pre_vote(&mut self) {
        if self.membership.learners.contains(&self.id) { return; }
        self.role = Role::PreCandidate;
        self.pre_votes_received.clear();
        self.pre_votes_received.insert(self.id);
        self.metrics.elections_started.inc();
        let candidate_term = self.current_term + 1;
        info!(id = self.id, term = candidate_term, "starting pre-vote");
        let msg = RaftMessage::RequestVote(RequestVote {
            term: candidate_term,
            candidate_id: self.id,
            last_log_index: self.log.last_index(),
            last_log_term: self.log.last_term(),
            pre_vote: true,
        });
        for peer_id in self.membership.all_voters().clone() {
            if peer_id != self.id { self.send(peer_id, msg.clone()).await; }
        }
        // Single-node: pre-vote trivially won.
        if self.membership.voters.len() == 1 {
            self.campaign_real().await;
        }
    }

    /// Initiate a real election (increments term, votes for self).
    async fn campaign_real(&mut self) {
        if self.membership.learners.contains(&self.id) { return; }
        let candidate_term = self.current_term + 1;
        self.become_candidate(candidate_term);
        info!(id = self.id, term = candidate_term, "starting real election");
        let msg = RaftMessage::RequestVote(RequestVote {
            term: candidate_term,
            candidate_id: self.id,
            last_log_index: self.log.last_index(),
            last_log_term: self.log.last_term(),
            pre_vote: false,
        });
        for peer_id in self.membership.all_voters().clone() {
            if peer_id != self.id { self.send(peer_id, msg.clone()).await; }
        }
        // Single-node cluster: immediately win.
        if self.membership.voters.len() == 1 {
            self.become_leader().await;
        }
    }

    async fn campaign(&mut self, pre_vote: bool) {
        if pre_vote {
            self.campaign_pre_vote().await;
        } else {
            self.campaign_real().await;
        }
    }

    fn become_candidate(&mut self, term: Term) {
        self.current_term = term;
        self.voted_for = Some(self.id);
        self.role = Role::Candidate;
        self.leader_id = None;
        self.votes_received.clear();
        self.votes_rejected.clear();
        self.votes_received.insert(self.id); // Vote for self.
        self.election_elapsed = 0;
        self.election_timeout = randomized_timeout(
            self.config.election_timeout_min,
            self.config.election_timeout_max,
        );
    }

    async fn become_leader(&mut self) {
        info!(id = self.id, term = self.current_term, "became leader");
        self.role = Role::Leader;
        self.leader_id = Some(self.id);
        self.heartbeat_elapsed = 0;
        self.check_quorum_elapsed = 0;
        self.transfer_to = None;
        self.transfer_elapsed = 0;
        self.lease.renew();
        self.metrics.is_leader.set(1i64);
        self.metrics.leader_id.set(self.id as i64);
        self.metrics.leader_changes.inc();

        // Initialize peer state.
        let next = self.log.last_index() + 1;
        self.peers.clear();
        for peer_id in self.membership.all_nodes() {
            if peer_id != self.id {
                self.peers.insert(peer_id, PeerState::new(next));
            }
        }

        // Append a no-op barrier entry to commit entries from previous terms.
        let barrier_index = self.log.last_index() + 1;
        let barrier = LogEntry::new_barrier(barrier_index, self.current_term);
        self.log.append(vec![barrier]);

        // Replicate immediately.
        self.broadcast_append_entries().await;
    }

    fn become_follower(&mut self, term: Term, leader_id: Option<NodeId>) {
        let was_leader = self.role == Role::Leader;
        if was_leader {
            self.metrics.is_leader.set(0i64);
            self.lease.invalidate();
            // Fail pending proposals.
            for (_, tx) in self.pending_proposals.drain() {
                let _ = tx.send(Err(HaError::NotLeader { leader_id }));
            }
            // Fail pending reads.
            for req in self.read_only.drain_all() {
                let _ = req.resp.send(Err(HaError::NotLeader { leader_id }));
            }
        }
        if term > self.current_term {
            self.current_term = term;
            self.voted_for = None;
        }
        self.role = Role::Follower;
        self.leader_id = leader_id;
        self.election_elapsed = 0;
        self.election_timeout = randomized_timeout(
            self.config.election_timeout_min,
            self.config.election_timeout_max,
        );
        self.votes_received.clear();
        self.pre_votes_received.clear();
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Step — process incoming Raft messages
    // ═══════════════════════════════════════════════════════════════════════

    async fn step(&mut self, from: NodeId, msg: RaftMessage) {
        let msg_term = msg.term();

        // Higher term: revert to follower.
        if msg_term > self.current_term {
            let leader = match &msg {
                RaftMessage::AppendEntries(m) => Some(m.leader_id),
                RaftMessage::InstallSnapshot(m) => Some(m.leader_id),
                _ => None,
            };
            // Don't revert on pre-vote (term is speculative).
            if !matches!(&msg, RaftMessage::RequestVote(rv) if rv.pre_vote) {
                self.become_follower(msg_term, leader);
            }
        }

        // Stale term: reject (except pre-vote replies).
        if msg_term < self.current_term {
            if !matches!(&msg, RaftMessage::RequestVoteReply(rv) if rv.pre_vote) {
                debug!(
                    id = self.id,
                    from,
                    msg_term,
                    current = self.current_term,
                    "dropping stale message"
                );
                return;
            }
        }

        match msg {
            RaftMessage::RequestVote(m) => self.handle_request_vote(from, m).await,
            RaftMessage::RequestVoteReply(m) => self.handle_request_vote_reply(from, m).await,
            RaftMessage::AppendEntries(m) => self.handle_append_entries(from, m).await,
            RaftMessage::AppendEntriesReply(m) => self.handle_append_entries_reply(from, m).await,
            RaftMessage::InstallSnapshot(m) => self.handle_install_snapshot(from, m).await,
            RaftMessage::InstallSnapshotReply(m) => self.handle_install_snapshot_reply(from, m).await,
            RaftMessage::TimeoutNow(m) => self.handle_timeout_now(from, m).await,
            RaftMessage::ReadIndexRequest(m) => self.handle_read_index_request(from, m).await,
            RaftMessage::ReadIndexReply(m) => self.handle_read_index_reply(from, m).await,
        }
    }

    // ── RequestVote ───────────────────────────────────────────────────────

    async fn handle_request_vote(&mut self, from: NodeId, msg: RequestVote) {
        let last_log_index = self.log.last_index();
        let last_log_term = self.log.last_term();
        let log_ok = msg.last_log_term > last_log_term
            || (msg.last_log_term == last_log_term && msg.last_log_index >= last_log_index);

        let grant = if msg.pre_vote {
            // Grant pre-vote if log is OK and we haven't heard from a leader recently.
            log_ok && self.election_elapsed >= self.election_timeout / 2
        } else {
            let can_vote = self.voted_for.is_none() || self.voted_for == Some(msg.candidate_id);
            log_ok && can_vote
        };

        if grant && !msg.pre_vote {
            self.voted_for = Some(msg.candidate_id);
            self.election_elapsed = 0; // Reset timer.
        }

        debug!(
            id = self.id,
            from,
            grant,
            pre_vote = msg.pre_vote,
            "RequestVote"
        );

        self.send(from, RaftMessage::RequestVoteReply(RequestVoteReply {
            term: self.current_term,
            vote_granted: grant,
            pre_vote: msg.pre_vote,
        })).await;
    }

    async fn handle_request_vote_reply(&mut self, from: NodeId, msg: RequestVoteReply) {
        if msg.pre_vote {
            if self.role != Role::PreCandidate {
                return;
            }
            if msg.vote_granted {
                self.pre_votes_received.insert(from);
                if self.membership.has_quorum(&self.pre_votes_received) {
                    // Pre-vote quorum: start real election.
                    self.campaign_real().await;
                }
            }
            return;
        }

        if self.role != Role::Candidate {
            return;
        }
        if msg.vote_granted {
            self.votes_received.insert(from);
            if self.membership.has_quorum(&self.votes_received) {
                self.become_leader().await;
            }
        } else {
            self.votes_rejected.insert(from);
            let voters = self.membership.voters.len();
            let rejections_needed = voters - MembershipConfig::quorum(voters) + 1;
            if self.votes_rejected.len() >= rejections_needed {
                // Definitely lost; back to follower.
                self.become_follower(self.current_term, None);
            }
        }
    }

    // ── AppendEntries ─────────────────────────────────────────────────────

    async fn handle_append_entries(&mut self, from: NodeId, msg: AppendEntries) {
        // Valid leader heartbeat: reset election timer.
        self.election_elapsed = 0;
        self.leader_id = Some(from);
        if self.role != Role::Follower {
            self.become_follower(msg.term, Some(from));
        }

        // Mark peer active for check-quorum if we're a leader (shouldn't happen here, but guard).
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.recent_active = true;
            peer.last_response = Instant::now();
        }

        // Consistency check.
        let (success, conflict_index, conflict_term) = self.check_log_consistency(&msg);
        if !success {
            self.send(from, RaftMessage::AppendEntriesReply(AppendEntriesReply {
                term: self.current_term,
                success: false,
                conflict_index,
                conflict_term,
                last_log_index: self.log.last_index(),
                seq: msg.seq,
            })).await;
            return;
        }

        // Append new entries.
        if !msg.entries.is_empty() {
            self.log.append(msg.entries);
        }

        // Advance commit index.
        if msg.leader_commit > self.commit_index {
            let new_commit = msg.leader_commit.min(self.log.last_index());
            self.advance_commit(new_commit).await;
        }

        self.send(from, RaftMessage::AppendEntriesReply(AppendEntriesReply {
            term: self.current_term,
            success: true,
            conflict_index: 0,
            conflict_term: None,
            last_log_index: self.log.last_index(),
            seq: msg.seq,
        })).await;
    }

    fn check_log_consistency(&self, msg: &AppendEntries) -> (bool, LogIndex, Option<Term>) {
        let prev = msg.prev_log_index;
        if prev == 0 {
            return (true, 0, None);
        }
        match self.log.term(prev) {
            Ok(t) if t == msg.prev_log_term => (true, 0, None),
            Ok(t) => {
                // Term mismatch: find first index of conflicting term.
                let conflict_index = self.first_index_of_term(t).unwrap_or(prev);
                (false, conflict_index, Some(t))
            }
            Err(_) => {
                // Don't have prev_log_index.
                (false, self.log.last_index() + 1, None)
            }
        }
    }

    fn first_index_of_term(&self, term: Term) -> Option<LogIndex> {
        let first = self.log.first_index();
        let last = self.log.last_index();
        for i in first..=last {
            if let Ok(t) = self.log.term(i) {
                if t == term {
                    return Some(i);
                }
            }
        }
        None
    }

    async fn handle_append_entries_reply(&mut self, from: NodeId, msg: AppendEntriesReply) {
        if self.role != Role::Leader {
            return;
        }
        if !self.peers.contains_key(&from) {
            return;
        }

        // Update peer state, then drop the borrow before calling async methods.
        let success = msg.success;
        {
            let peer = self.peers.get_mut(&from).unwrap();
            // Decrement pipeline inflight count.
            peer.inflight = peer.inflight.saturating_sub(1);
            peer.last_response = Instant::now();
            peer.recent_active = true;

            if success {
                peer.match_index = peer.match_index.max(msg.last_log_index);
                peer.next_index = peer.match_index + 1;
            } else {
                // Fast log backtracking.
                if let Some(ct) = msg.conflict_term {
                    let last_match = {
                        let mut found = None;
                        let first = self.log.first_index();
                        let last = self.log.last_index();
                        for i in (first..=last).rev() {
                            if let Ok(t) = self.log.term(i) {
                                if t == ct { found = Some(i); break; }
                                if t < ct { break; }
                            }
                        }
                        found
                    };
                    peer.next_index = last_match.map(|i| i + 1).unwrap_or(msg.conflict_index);
                } else {
                    peer.next_index = msg.conflict_index.max(1);
                }
                peer.next_index = peer.next_index.max(1);
            }
        }

        if success {
            self.maybe_advance_commit().await;
            self.replicate_to(from).await;
            self.maybe_flush_reads().await;
        } else {
            self.replicate_to(from).await;
        }
    }

    // ── Replication helpers ───────────────────────────────────────────────

    async fn broadcast_heartbeat(&mut self) {
        self.metrics.heartbeats_sent.inc();
        self.lease.renew();
        let peer_ids: Vec<NodeId> = self.peers.keys().copied().collect();
        for peer_id in peer_ids {
            self.replicate_to(peer_id).await;
        }
    }

    async fn broadcast_append_entries(&mut self) {
        let peer_ids: Vec<NodeId> = self.peers.keys().copied().collect();
        for peer_id in peer_ids {
            self.replicate_to(peer_id).await;
        }
    }

    async fn replicate_to(&mut self, peer_id: NodeId) {
        let peer = match self.peers.get_mut(&peer_id) {
            Some(p) => p,
            None => return,
        };

        if peer.snapshot_in_progress {
            return;
        }
        if peer.inflight >= self.config.pipeline_depth as u64 {
            return; // Flow control: pipeline full.
        }

        let next = peer.next_index;

        // If peer is behind snapshot, send snapshot instead.
        if next <= self.log.snapshot_index() {
            peer.snapshot_in_progress = true;
            // Trigger snapshot send (simplified: build snapshot and send).
            self.send_snapshot_to(peer_id).await;
            return;
        }

        let prev_index = next - 1;
        let prev_term = self.log.term(prev_index).unwrap_or(0);
        let last_index = self.log.last_index();
        let hi = (next + self.config.max_append_entries as u64).min(last_index + 1);

        let entries = if next <= last_index {
            self.log.slice(next, hi).unwrap_or_default()
        } else {
            vec![]
        };

        let peer = self.peers.get_mut(&peer_id).unwrap();
        let seq = peer.next_seq;
        peer.next_seq += 1;
        if !entries.is_empty() {
            peer.inflight += 1;
        }

        self.metrics.append_entries_sent.inc();
        let msg = RaftMessage::AppendEntries(AppendEntries {
            term: self.current_term,
            leader_id: self.id,
            prev_log_index: prev_index,
            prev_log_term: prev_term,
            entries,
            leader_commit: self.commit_index,
            seq,
        });
        self.send(peer_id, msg).await;
    }

    async fn send_snapshot_to(&mut self, peer_id: NodeId) {
        // Build a snapshot from current state machine.
        let data = match self.state_machine.snapshot().await {
            Ok(d) => d,
            Err(e) => {
                warn!("snapshot failed: {e}");
                if let Some(p) = self.peers.get_mut(&peer_id) {
                    p.snapshot_in_progress = false;
                }
                return;
            }
        };
        let snapshot = Snapshot::new(
            self.log.snapshot_index(),
            self.log.snapshot_term(),
            self.membership.clone(),
            data,
        );
        let chunks = snapshot.chunks(self.config.snapshot_chunk_size);
        self.metrics.snapshots_sent.inc();
        for chunk in chunks {
            let msg = RaftMessage::InstallSnapshot(InstallSnapshot {
                term: self.current_term,
                leader_id: self.id,
                meta: chunk.meta,
                offset: chunk.offset,
                data: chunk.data,
                done: chunk.done,
            });
            self.send(peer_id, msg).await;
        }
    }

    // ── InstallSnapshot ───────────────────────────────────────────────────

    async fn handle_install_snapshot(&mut self, from: NodeId, msg: InstallSnapshot) {
        self.election_elapsed = 0;
        self.leader_id = Some(from);

        let recv = self.snapshot_receiver.get_or_insert_with(SnapshotReceiver::new);
        let chunk = crate::raft::snapshot::SnapshotChunk {
            meta: msg.meta.clone(),
            offset: msg.offset,
            data: msg.data,
            done: msg.done,
        };
        let bytes_stored = chunk.offset + chunk.data.len() as u64;

        if let Some(snapshot) = recv.feed(chunk) {
            self.snapshot_receiver = None;
            self.metrics.snapshots_received.inc();
            self.apply_snapshot(snapshot).await;
        }

        self.send(from, RaftMessage::InstallSnapshotReply(InstallSnapshotReply {
            term: self.current_term,
            success: true,
            bytes_stored,
        })).await;
    }

    async fn apply_snapshot(&mut self, snapshot: Snapshot) {
        let meta = &snapshot.meta;
        if meta.index <= self.commit_index {
            return; // Already have a more recent state.
        }
        if let Err(e) = self.state_machine.restore(&snapshot.data).await {
            warn!("snapshot restore failed: {e}");
            return;
        }
        self.log.compact(meta.index, meta.term);
        self.commit_index = meta.index;
        self.last_applied = meta.index;
        self.membership = meta.membership.clone();
        self.metrics.commit_index.set(meta.index as i64);
        self.metrics.last_applied.set(meta.index as i64);
        info!(id = self.id, index = meta.index, "applied snapshot");
    }

    async fn handle_install_snapshot_reply(&mut self, from: NodeId, msg: InstallSnapshotReply) {
        if self.role != Role::Leader { return; }
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.snapshot_in_progress = false;
            if msg.success {
                // Assume snapshot fully installed; advance match/next.
                let snap_idx = self.log.snapshot_index();
                peer.match_index = snap_idx;
                peer.next_index = snap_idx + 1;
            }
        }
        self.maybe_advance_commit().await;
    }

    // ── TimeoutNow ────────────────────────────────────────────────────────

    async fn handle_timeout_now(&mut self, _from: NodeId, msg: TimeoutNow) {
        if self.membership.learners.contains(&self.id) { return; }
        info!(id = self.id, from = msg.from, "TimeoutNow: starting immediate election");
        self.election_elapsed = self.election_timeout; // Force immediate election.
        self.campaign(self.config.pre_vote).await;
    }

    // ── ReadIndex ─────────────────────────────────────────────────────────

    async fn handle_read_index_request(&mut self, from: NodeId, msg: ReadIndexRequest) {
        if self.role != Role::Leader {
            self.send(from, RaftMessage::ReadIndexReply(ReadIndexReply {
                term: self.current_term,
                id: msg.id,
                read_index: 0,
                success: false,
            })).await;
            return;
        }
        // LeaseRead: if lease is valid, reply immediately.
        if self.read_only.mode() == ReadMode::LeaseRead && self.lease.is_valid() {
            self.send(from, RaftMessage::ReadIndexReply(ReadIndexReply {
                term: self.current_term,
                id: msg.id,
                read_index: self.commit_index,
                success: true,
            })).await;
            return;
        }
        // ReadIndex: respond when heartbeat confirms quorum.
        // Store the request; it'll be resolved in maybe_flush_reads.
        // For cross-node: just reply immediately with current commit (simplified).
        self.send(from, RaftMessage::ReadIndexReply(ReadIndexReply {
            term: self.current_term,
            id: msg.id,
            read_index: self.commit_index,
            success: true,
        })).await;
    }

    async fn handle_read_index_reply(&mut self, _from: NodeId, _msg: ReadIndexReply) {
        // For followers that forwarded: resolve waiting client.
    }

    // ── Commit & apply ────────────────────────────────────────────────────

    async fn maybe_advance_commit(&mut self) {
        if self.role != Role::Leader { return; }
        let last = self.log.last_index();
        let mut new_commit = self.commit_index;
        for idx in (self.commit_index + 1)..=last {
            // Only commit entries from current term (Raft safety).
            if let Ok(term) = self.log.term(idx) {
                if term != self.current_term { continue; }
            } else {
                continue;
            }
            // Count replications.
            let mut votes: BTreeSet<NodeId> = BTreeSet::new();
            votes.insert(self.id);
            for (pid, p) in &self.peers {
                if p.match_index >= idx {
                    votes.insert(*pid);
                }
            }
            if self.membership.has_quorum(&votes) {
                new_commit = idx;
            }
        }
        if new_commit > self.commit_index {
            self.advance_commit(new_commit).await;
        }
    }

    async fn advance_commit(&mut self, new_commit: LogIndex) {
        self.commit_index = new_commit;
        self.metrics.commit_index.set(new_commit as i64);
        self.apply_entries().await;

        // Notify proposals that are now committed.
        let mut resolved = vec![];
        for (idx, _) in &self.pending_proposals {
            if *idx <= new_commit {
                resolved.push(*idx);
            }
        }
        for idx in resolved {
            if let Some(tx) = self.pending_proposals.remove(&idx) {
                let _ = tx.send(Ok(idx));
            }
        }

        // Check if auto-leave joint consensus.
        if self.role == Role::Leader && self.membership.auto_leave && !self.membership.is_joint() {
            // Already transitioned; nothing to do.
        }
    }

    async fn apply_entries(&mut self) {
        while self.last_applied < self.commit_index {
            self.last_applied += 1;
            let entry = match self.log.entry(self.last_applied) {
                Ok(e) => e.clone(),
                Err(_) => break,
            };
            match entry.entry_type {
                EntryType::Normal | EntryType::Barrier => {
                    let kernel_entry = crate::raft::kernel_bridge::to_kernel_entry(&entry);
                    if let Err(e) = self.state_machine.apply(&kernel_entry).await {
                        warn!(id = self.id, index = self.last_applied, "apply error: {e}");
                    }
                }
                EntryType::MembershipChange => {
                    if let Ok(new_cfg) = entry.decode_membership() {
                        self.membership = new_cfg;
                        // If joint config with auto_leave, append C_new immediately.
                        if self.membership.auto_leave && self.membership.is_joint() && self.role == Role::Leader {
                            let next_cfg = MembershipConfig {
                                voters: self.membership.voters.clone(),
                                learners: self.membership.learners.clone(),
                                voters_outgoing: None,
                                auto_leave: false,
                            };
                            self.append_membership_change(next_cfg).await;
                        }
                        self.metrics.membership_changes.inc();
                    }
                }
            }
            self.metrics.entries_applied.inc();
        }
        self.metrics.last_applied.set(self.last_applied as i64);

        // Trigger compaction if log is too large.
        if self.log.len() as u64 > self.config.log_compaction_threshold {
            self.compact_log().await;
        }
    }

    async fn maybe_flush_reads(&mut self) {
        if self.read_only.is_empty() { return; }
        let quorum = MembershipConfig::quorum(self.membership.voters.len());
        // Count active peers (including self).
        let active = self.peers.values().filter(|p| p.recent_active).count() + 1;
        if active >= quorum {
            let ready = self.read_only.ack(self.id, quorum);
            for req in ready {
                if self.last_applied >= req.index {
                    let _ = req.resp.send(Ok(req.index));
                } else {
                    // Re-queue: not yet applied.
                    let _ = req.resp.send(Ok(req.index));
                }
            }
        }
    }

    // ── Log compaction ────────────────────────────────────────────────────

    async fn compact_log(&mut self) {
        let compact_to = self.last_applied;
        if compact_to <= self.log.snapshot_index() { return; }
        let term = self.log.term(compact_to).unwrap_or(self.current_term);
        self.log.compact(compact_to, term);
        self.metrics.log_compactions.inc();

        if let Some(tx) = self.pending_snapshot_resp.take() {
            let meta = SnapshotMeta {
                index: compact_to,
                term,
                membership: self.membership.clone(),
            };
            let _ = tx.send(Ok(meta));
        }
    }

    // ── Check quorum ──────────────────────────────────────────────────────

    async fn do_check_quorum(&mut self) {
        if self.role != Role::Leader { return; }
        let quorum = MembershipConfig::quorum(self.membership.voters.len());
        let active = self.peers.values().filter(|p| p.recent_active).count() + 1;
        // Reset activity flags for next round.
        for p in self.peers.values_mut() {
            p.recent_active = false;
        }
        if active < quorum {
            warn!(
                id = self.id,
                active,
                quorum,
                "check-quorum failed: stepping down"
            );
            self.metrics.check_quorum_failures.inc();
            self.become_follower(self.current_term, None);
        }
    }

    // ── Membership changes ────────────────────────────────────────────────

    async fn append_membership_change(&mut self, cfg: MembershipConfig) {
        let index = self.log.last_index() + 1;
        if let Ok(entry) = LogEntry::new_membership(index, self.current_term, &cfg) {
            self.log.append(vec![entry]);
            self.broadcast_append_entries().await;
        }
    }

    // ── Leadership transfer ───────────────────────────────────────────────

    async fn initiate_transfer(&mut self, to: NodeId) -> HaResult<()> {
        if self.role != Role::Leader {
            return Err(HaError::NotLeader { leader_id: self.leader_id });
        }
        if !self.membership.voters.contains(&to) {
            return Err(HaError::NodeNotFound(to));
        }
        self.transfer_to = Some(to);
        self.transfer_elapsed = 0;
        // Catch up the target first.
        self.replicate_to(to).await;
        // Send TimeoutNow if target is caught up.
        let target_match = self.peers.get(&to).map(|p| p.match_index).unwrap_or(0);
        if target_match >= self.log.last_index() {
            self.send(to, RaftMessage::TimeoutNow(TimeoutNow {
                term: self.current_term,
                from: self.id,
            })).await;
        }
        Ok(())
    }

    // ── Command handling ──────────────────────────────────────────────────

    /// Returns true if node should shut down.
    async fn handle_cmd(&mut self, cmd: NodeCmd) -> bool {
        match cmd {
            NodeCmd::Propose { data, resp } => {
                if self.role != Role::Leader {
                    let _ = resp.send(Err(HaError::NotLeader { leader_id: self.leader_id }));
                    self.metrics.proposals_failed.inc();
                    return false;
                }
                if self.transfer_to.is_some() {
                    let _ = resp.send(Err(HaError::TransferInProgress));
                    return false;
                }
                self.metrics.proposals_total.inc();
                let index = self.log.last_index() + 1;
                let entry = LogEntry::new_normal(index, self.current_term, data);
                self.log.append(vec![entry]);
                self.pending_proposals.insert(index, resp);
                self.broadcast_append_entries().await;
                // Single-node fast path.
                if self.membership.voters.len() == 1 {
                    self.advance_commit(index).await;
                }
            }
            NodeCmd::ReadIndex { resp } => {
                if self.role != Role::Leader {
                    let _ = resp.send(Err(HaError::NotLeader { leader_id: self.leader_id }));
                    return false;
                }
                self.metrics.read_index_requests.inc();
                if self.read_only.mode() == ReadMode::LeaseRead && self.lease.is_valid() {
                    let _ = resp.send(Ok(self.commit_index));
                    return false;
                }
                // Queue a ReadIndex request.
                self.read_only.add(self.commit_index, resp);
                // Broadcast heartbeat to confirm leadership.
                self.broadcast_heartbeat().await;
            }
            NodeCmd::AddNode { node, resp } => {
                if self.role != Role::Leader {
                    let _ = resp.send(Err(HaError::NotLeader { leader_id: self.leader_id }));
                    return false;
                }
                // Joint consensus: C_old,new
                let mut new_voters = self.membership.voters.clone();
                let mut new_learners = self.membership.learners.clone();
                if node.is_learner {
                    new_learners.insert(node.id);
                } else {
                    new_learners.remove(&node.id);
                    new_voters.insert(node.id);
                }
                let joint_cfg = MembershipConfig {
                    voters: new_voters,
                    learners: new_learners,
                    voters_outgoing: Some(self.membership.voters.clone()),
                    auto_leave: true,
                };
                self.append_membership_change(joint_cfg).await;
                // Add peer state.
                let next = self.log.last_index() + 1;
                self.peers.entry(node.id).or_insert_with(|| PeerState::new(next));
                let _ = resp.send(Ok(()));
            }
            NodeCmd::RemoveNode { id, resp } => {
                if self.role != Role::Leader {
                    let _ = resp.send(Err(HaError::NotLeader { leader_id: self.leader_id }));
                    return false;
                }
                let mut new_voters = self.membership.voters.clone();
                new_voters.remove(&id);
                let new_learners = self.membership.learners.clone();
                let joint_cfg = MembershipConfig {
                    voters: new_voters,
                    learners: new_learners,
                    voters_outgoing: Some(self.membership.voters.clone()),
                    auto_leave: true,
                };
                self.append_membership_change(joint_cfg).await;
                let _ = resp.send(Ok(()));
                // If removing self, step down after commit.
                if id == self.id {
                    self.become_follower(self.current_term, None);
                }
            }
            NodeCmd::TransferLeadership { to, resp } => {
                let result = self.initiate_transfer(to).await;
                let _ = resp.send(result);
            }
            NodeCmd::TriggerSnapshot { resp } => {
                self.pending_snapshot_resp = Some(resp);
                self.compact_log().await;
            }
            NodeCmd::Status { resp } => {
                let status = NodeStatus {
                    id: self.id,
                    role: self.role.to_string(),
                    term: self.current_term,
                    commit_index: self.commit_index,
                    last_applied: self.last_applied,
                    leader_id: self.leader_id,
                    membership: self.membership.clone(),
                    last_log_index: self.log.last_index(),
                    last_log_term: self.log.last_term(),
                };
                let _ = resp.send(status);
            }
            NodeCmd::Shutdown => return true,
        }
        false
    }

    // ── Transport helpers ─────────────────────────────────────────────────

    async fn send(&self, to: NodeId, msg: RaftMessage) {
        if let Err(e) = self.transport.send(to, msg).await {
            debug!(id = self.id, to, "send failed: {e}");
        }
    }
}

// ── Private helpers ───────────────────────────────────────────────────────

fn randomized_timeout(min: u64, max: u64) -> u64 {
    use rand::Rng;
    rand::thread_rng().gen_range(min..=max)
}

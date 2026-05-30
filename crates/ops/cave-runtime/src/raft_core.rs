// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Raft consensus state machine.
//!
//! This module implements the **safety-critical** core of Raft from
//! "In Search of an Understandable Consensus Algorithm" (Ongaro &
//! Ousterhout, 2014), §5: leader election + log replication. The
//! transport layer ([`crate::raft_transport`]) frames the wire
//! envelopes; this module decides what to do with them.
//!
//! ## Scope
//!
//! Implemented:
//! * Persistent state (`current_term`, `voted_for`, `log`) with fsync
//!   on every change (paper Figure 2).
//! * Safe `RequestVote` handling: log-up-to-date check (§5.4.1),
//!   single-vote-per-term invariant, term-update-and-step-down on
//!   higher peer terms.
//! * Election driver: randomized election timeout (150-300 ms), term
//!   increment on timeout, RequestVote round, majority → leader.
//! * `AppendEntries` handling: prev-log consistency check (§5.3),
//!   follower truncation on mismatch, append + commit advancement.
//! * Leader log replication: per-peer `next_index` / `match_index`
//!   tracking, decrement-on-mismatch backtracking, majority match
//!   → commit advancement.
//! * `propose`: append-to-log for leader, returns assigned index;
//!   `NotLeader` for follower / candidate.
//! * Committed-entry stream the host can drain (`take_committed_entries`).
//!
//! Deliberately deferred (called out per the session brief — pursuing
//! these without separate runway risks shipping unsafe code):
//! * Pre-vote round (paper §9.6) — stability optimization, not safety.
//! * Joint-consensus reconfiguration (§6).
//! * `InstallSnapshot` RPC + log compaction (§7).
//! * Linearizable read index (§8) and leader leases.
//! * Wiring committed entries through to `cave-etcd::KvStore` /
//!   `cave-apiserver::ResourceStore` state machines.
//!
//! The `propose` → commit → drain path is exercised by tests against
//! an in-process command stream; the host can subscribe via
//! `take_committed_entries` and decide what to apply.
//!
//! ## Test strategy
//!
//! Unit tests drive `RaftCore::tick` with a synthetic clock and pipe
//! outbound messages between core instances manually. This catches
//! every state-transition decision deterministically without the
//! flakiness inherent in real-network tests. The 3-node smoke at the
//! end of the session exercises the same paths against the live
//! transport.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub type NodeId = u64;
pub type Term = u64;
pub type LogIndex = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Follower,
    Candidate,
    Leader,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogEntry {
    pub term: Term,
    pub index: LogIndex,
    /// Opaque application command. The host decides how to interpret
    /// it once committed.
    pub command: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistentState {
    pub current_term: Term,
    pub voted_for: Option<NodeId>,
    /// 1-indexed (Raft convention). `log[i-1].index == i`. Entry 0 is
    /// a placeholder "before-the-log" record with term 0 — never sent.
    pub log: Vec<LogEntry>,
}

// ── Wire-level RPC payloads ────────────────────────────────────────────────
// These mirror the paper's Figure 2 RPC arguments and results.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestVoteArgs {
    pub term: Term,
    pub candidate_id: NodeId,
    pub last_log_index: LogIndex,
    pub last_log_term: Term,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestVoteReply {
    pub term: Term,
    pub vote_granted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesArgs {
    pub term: Term,
    pub leader_id: NodeId,
    pub prev_log_index: LogIndex,
    pub prev_log_term: Term,
    pub entries: Vec<LogEntry>,
    pub leader_commit: LogIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesReply {
    pub term: Term,
    pub success: bool,
    /// Optimization: if `success = false`, the follower may report the
    /// highest index it does have so the leader can skip multiple
    /// `next_index` decrements. `0` means "fall back one at a time".
    pub conflict_index: LogIndex,
}

/// One outbound RPC the core decided to emit. The transport layer
/// fans these out (parallel POSTs in production; an explicit
/// pipe in tests).
#[derive(Debug, Clone)]
pub struct Outbound {
    pub to: NodeId,
    pub msg: OutboundMessage,
}

#[derive(Debug, Clone)]
pub enum OutboundMessage {
    RequestVote(RequestVoteArgs),
    AppendEntries(AppendEntriesArgs),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProposeError {
    #[error("local node is not leader (current role: {0:?}, leader: {1:?})")]
    NotLeader(Role, Option<NodeId>),
}

// ── Core ───────────────────────────────────────────────────────────────────

pub struct RaftCore {
    // Persistent (Figure 2): fsync'd to `state_path` on every mutation.
    pub persistent: PersistentState,
    pub state_path: PathBuf,

    // Volatile, all-roles.
    pub role: Role,
    pub commit_index: LogIndex,
    pub last_applied: LogIndex,
    pub leader_id: Option<NodeId>,
    pub local_id: NodeId,
    /// All node ids in the cluster INCLUDING local. Stable for the life
    /// of this core (reconfig is deferred).
    pub members: Vec<NodeId>,

    // Election state — valid in Candidate role.
    votes_received: HashSet<NodeId>,

    // Leader-only state, reset every time the local node becomes leader.
    next_index: BTreeMap<NodeId, LogIndex>,
    match_index: BTreeMap<NodeId, LogIndex>,

    // Timer state.
    election_deadline: Instant,
    /// Most recent randomized election timeout — captured here so tests
    /// can inspect it without re-rolling the RNG.
    election_timeout: Duration,
    /// Min/max bounds for the randomized election timeout.
    timeout_range_ms: (u64, u64),
    /// Leader heartbeat interval. Must be << election_timeout_min.
    heartbeat_interval: Duration,
    /// Next deadline at which the leader fans out heartbeats. `now` advances
    /// it on each tick when in Leader role.
    next_heartbeat_deadline: Instant,

    // Test affordances.
    /// Deterministic RNG seed for the randomized election timeout. Real
    /// nodes set this to a node-id-mixed system random value.
    rng_state: u64,
}

impl RaftCore {
    /// Build a fresh core or load the persistent state from disk. If
    /// `<data_dir>/raft/state.bin` exists and parses, it's adopted;
    /// otherwise a clean state is started.
    pub fn load_or_init(
        local_id: NodeId,
        members: Vec<NodeId>,
        data_dir: &Path,
        now: Instant,
    ) -> Result<Self> {
        let state_path = data_dir.join("raft").join("state.bin");
        let persistent = if state_path.exists() {
            let bytes = std::fs::read(&state_path)
                .with_context(|| format!("read {}", state_path.display()))?;
            match serde_json::from_slice::<PersistentState>(&bytes) {
                Ok(s) => {
                    info!(
                        path = %state_path.display(),
                        term = s.current_term,
                        log_len = s.log.len(),
                        "raft persistent state restored"
                    );
                    s
                }
                Err(e) => {
                    warn!(error = %e, "raft state.bin parse failed — starting fresh");
                    PersistentState::default()
                }
            }
        } else {
            PersistentState::default()
        };

        let rng_state = derive_rng_seed(local_id);
        let mut core = Self {
            persistent,
            state_path,
            role: Role::Follower,
            commit_index: 0,
            last_applied: 0,
            leader_id: None,
            local_id,
            members,
            votes_received: HashSet::new(),
            next_index: BTreeMap::new(),
            match_index: BTreeMap::new(),
            election_deadline: now,
            election_timeout: Duration::from_millis(0),
            timeout_range_ms: (150, 300),
            heartbeat_interval: Duration::from_millis(50),
            next_heartbeat_deadline: now,
            rng_state,
        };
        core.reset_election_deadline(now);
        Ok(core)
    }

    /// Build a fully-in-memory core for tests. Persistent writes still
    /// hit the supplied path (use a `tempdir`).
    pub fn new_for_test(
        local_id: NodeId,
        members: Vec<NodeId>,
        data_dir: &Path,
        now: Instant,
    ) -> Self {
        Self::load_or_init(local_id, members, data_dir, now)
            .expect("test core build should succeed in tempdir")
    }

    // ── Persistent-state primitives ────────────────────────────────────────

    /// Persist `current_term` + `voted_for` + `log` atomically. fsync'd.
    /// Returns error if the write or rename fails — callers should panic
    /// rather than continue with stale durability assumptions.
    pub fn save_persistent(&self) -> Result<()> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let bytes = serde_json::to_vec(&self.persistent).context("encode raft PersistentState")?;
        let tmp_path = self.state_path.with_extension("bin.tmp");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)
            .with_context(|| format!("open {}", tmp_path.display()))?;
        use std::io::Write;
        f.write_all(&bytes).context("write raft state")?;
        f.sync_data().context("fsync raft state")?;
        drop(f);
        std::fs::rename(&tmp_path, &self.state_path).with_context(|| {
            format!(
                "rename {} -> {}",
                tmp_path.display(),
                self.state_path.display()
            )
        })?;
        Ok(())
    }

    fn set_term_and_persist(&mut self, term: Term) -> Result<()> {
        if term != self.persistent.current_term {
            self.persistent.current_term = term;
            self.persistent.voted_for = None;
        }
        self.save_persistent()
    }

    fn record_vote_and_persist(&mut self, for_node: NodeId) -> Result<()> {
        self.persistent.voted_for = Some(for_node);
        self.save_persistent()
    }

    // ── Log primitives ─────────────────────────────────────────────────────

    pub fn last_log_index(&self) -> LogIndex {
        self.persistent.log.last().map(|e| e.index).unwrap_or(0)
    }

    pub fn last_log_term(&self) -> Term {
        self.persistent.log.last().map(|e| e.term).unwrap_or(0)
    }

    /// Returns the entry whose `index == idx`, if any. 1-indexed.
    pub fn log_at(&self, idx: LogIndex) -> Option<&LogEntry> {
        if idx == 0 {
            return None;
        }
        // log is 0-based, entries are 1-based; we assume contiguous from index 1.
        self.persistent.log.get((idx - 1) as usize)
    }

    /// Truncates the log so that the highest remaining index is `up_to`.
    fn truncate_log_to(&mut self, up_to: LogIndex) {
        let new_len = up_to as usize;
        if self.persistent.log.len() > new_len {
            self.persistent.log.truncate(new_len);
        }
    }

    fn append_entries_to_log(&mut self, entries: &[LogEntry]) {
        for e in entries {
            // Either replacing a prior entry at this index or appending fresh.
            let pos = (e.index - 1) as usize;
            if pos < self.persistent.log.len() {
                self.persistent.log[pos] = e.clone();
            } else {
                // Caller guarantees contiguity; if not, this is a bug.
                debug_assert_eq!(pos, self.persistent.log.len());
                self.persistent.log.push(e.clone());
            }
        }
    }

    // ── Role transitions ───────────────────────────────────────────────────

    fn become_follower(&mut self, term: Term, leader: Option<NodeId>, now: Instant) -> Result<()> {
        self.role = Role::Follower;
        self.leader_id = leader;
        self.votes_received.clear();
        self.set_term_and_persist(term)?;
        self.reset_election_deadline(now);
        Ok(())
    }

    fn become_candidate(&mut self, now: Instant) -> Result<()> {
        self.role = Role::Candidate;
        self.leader_id = None;
        // Increment term, vote for self, reset election timer.
        let new_term = self.persistent.current_term + 1;
        self.persistent.current_term = new_term;
        self.persistent.voted_for = Some(self.local_id);
        self.save_persistent()?;
        self.votes_received.clear();
        self.votes_received.insert(self.local_id);
        self.reset_election_deadline(now);
        info!(node = self.local_id, term = new_term, "→ Candidate");
        Ok(())
    }

    fn become_leader(&mut self, now: Instant) {
        self.role = Role::Leader;
        self.leader_id = Some(self.local_id);
        let last = self.last_log_index();
        self.next_index.clear();
        self.match_index.clear();
        for id in &self.members {
            if *id == self.local_id {
                continue;
            }
            // Paper Figure 2: next_index initialised to leader's last_log_index + 1.
            self.next_index.insert(*id, last + 1);
            self.match_index.insert(*id, 0);
        }
        // Force an immediate heartbeat by setting the deadline to `now`.
        self.next_heartbeat_deadline = now;
        info!(
            node = self.local_id,
            term = self.persistent.current_term,
            "→ Leader"
        );
    }

    // ── Election timing ────────────────────────────────────────────────────

    fn reset_election_deadline(&mut self, now: Instant) {
        let (lo, hi) = self.timeout_range_ms;
        let span = hi.saturating_sub(lo).max(1);
        // xorshift step.
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        let pick = lo + (self.rng_state % span);
        self.election_timeout = Duration::from_millis(pick);
        self.election_deadline = now + self.election_timeout;
    }

    pub fn election_deadline(&self) -> Instant {
        self.election_deadline
    }

    pub fn election_timeout(&self) -> Duration {
        self.election_timeout
    }

    /// Test-only: jump straight into Candidate state. Production code
    /// triggers this via the election timeout in `tick`.
    pub fn become_candidate_for_test(&mut self, now: Instant) {
        self.become_candidate(now).expect("test-only");
    }

    /// Test-only: jump straight into Leader state. Production code
    /// triggers this via majority vote in `handle_request_vote_reply`.
    pub fn become_leader_for_test(&mut self, now: Instant) {
        self.become_leader(now);
    }

    /// Allow tests / hosts to tune timing. `(150, 300)` is the paper's
    /// recommendation for a typical LAN; bump it on slow networks.
    pub fn set_timeout_range_ms(&mut self, lo: u64, hi: u64) {
        self.timeout_range_ms = (lo.max(1), hi.max(lo + 1));
    }

    pub fn set_heartbeat_interval(&mut self, d: Duration) {
        self.heartbeat_interval = d;
    }

    // ── Public state queries ───────────────────────────────────────────────

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn current_term(&self) -> Term {
        self.persistent.current_term
    }

    pub fn leader(&self) -> Option<NodeId> {
        self.leader_id
    }

    pub fn voted_for(&self) -> Option<NodeId> {
        self.persistent.voted_for
    }

    pub fn commit_index(&self) -> LogIndex {
        self.commit_index
    }

    pub fn last_applied(&self) -> LogIndex {
        self.last_applied
    }

    pub fn log_len(&self) -> usize {
        self.persistent.log.len()
    }

    pub fn members(&self) -> &[NodeId] {
        &self.members
    }

    fn majority(&self) -> usize {
        self.members.len() / 2 + 1
    }

    /// Resolve the current election outcome through the ported etcd
    /// [`ProgressTracker`](cave_etcd::raft_tracker::ProgressTracker) vote ledger.
    ///
    /// The candidate's granted votes (including its own self-vote, recorded in
    /// `votes_received`) are tallied against the voter configuration. For the
    /// single (non-joint) config this is behaviourally identical to
    /// `votes_received.len() >= majority()`, but routes the decision through the
    /// joint-aware `VoteResult` primitive so a future joint reconfiguration is
    /// handled correctly.
    pub fn election_vote_result(&self) -> cave_etcd::raft_tracker::VoteResult {
        let mut tracker = cave_etcd::raft_tracker::ProgressTracker::new(1);
        for &m in &self.members {
            tracker.add_voter(m);
        }
        for &granted in &self.votes_received {
            tracker.record_vote(granted, true);
        }
        tracker.vote_result()
    }

    // ── Inbound RPC handlers ───────────────────────────────────────────────

    /// Process a `RequestVote` from a candidate. Per paper §5.4.1, grant
    /// vote iff:
    ///   1. `args.term >= current_term` (callers update term first).
    ///   2. `voted_for` is `None` or `args.candidate_id`.
    ///   3. Candidate's log is at least as up-to-date as ours.
    pub fn handle_request_vote(
        &mut self,
        args: RequestVoteArgs,
        now: Instant,
    ) -> Result<RequestVoteReply> {
        // 1. Stale-term short circuit (Figure 2: §5.1).
        if args.term < self.persistent.current_term {
            return Ok(RequestVoteReply {
                term: self.persistent.current_term,
                vote_granted: false,
            });
        }
        // 2. If we see a higher term, step down to follower first.
        if args.term > self.persistent.current_term {
            self.become_follower(args.term, None, now)?;
        }
        // 3. Single-vote-per-term.
        let already_voted_for_other = matches!(
            self.persistent.voted_for,
            Some(other) if other != args.candidate_id
        );
        if already_voted_for_other {
            return Ok(RequestVoteReply {
                term: self.persistent.current_term,
                vote_granted: false,
            });
        }
        // 4. Log-up-to-date check (§5.4.1).
        if !candidate_log_at_least_as_up_to_date(
            args.last_log_term,
            args.last_log_index,
            self.last_log_term(),
            self.last_log_index(),
        ) {
            return Ok(RequestVoteReply {
                term: self.persistent.current_term,
                vote_granted: false,
            });
        }
        // Grant.
        self.record_vote_and_persist(args.candidate_id)?;
        // Granting a vote resets the election timer (Figure 2 footnote +
        // §5.2): otherwise a follower that just voted might immediately
        // time out and disrupt the candidate.
        self.reset_election_deadline(now);
        Ok(RequestVoteReply {
            term: self.persistent.current_term,
            vote_granted: true,
        })
    }

    /// Process an `AppendEntries` from a leader. Per paper §5.3:
    ///   1. Reject if `args.term < current_term`.
    ///   2. If `args.term > current_term`, step down.
    ///   3. If we don't have a matching entry at `prev_log_index` with
    ///      `prev_log_term`, reject with a conflict_index hint.
    ///   4. Truncate any conflicting suffix, append new entries.
    ///   5. Advance `commit_index` up to `min(leader_commit, last_log_index)`.
    pub fn handle_append_entries(
        &mut self,
        args: AppendEntriesArgs,
        now: Instant,
    ) -> Result<AppendEntriesReply> {
        // 1. Reject stale.
        if args.term < self.persistent.current_term {
            return Ok(AppendEntriesReply {
                term: self.persistent.current_term,
                success: false,
                conflict_index: 0,
            });
        }
        // 2. Step down on higher (or matching) term.
        if args.term > self.persistent.current_term
            || (self.role == Role::Candidate && args.term == self.persistent.current_term)
        {
            self.become_follower(args.term, Some(args.leader_id), now)?;
        }
        // Note: even at equal term we recognise the leader.
        self.leader_id = Some(args.leader_id);
        self.role = Role::Follower;
        self.reset_election_deadline(now);

        // 3. Prev-log consistency check.
        if args.prev_log_index > 0 {
            match self.log_at(args.prev_log_index) {
                Some(e) if e.term == args.prev_log_term => {
                    // match — fall through
                }
                Some(_e) => {
                    // Term mismatch at prev. Report hint = last index we have
                    // with term <= prev_log_term so leader can skip back.
                    return Ok(AppendEntriesReply {
                        term: self.persistent.current_term,
                        success: false,
                        conflict_index: args.prev_log_index.saturating_sub(1),
                    });
                }
                None => {
                    // Hole in our log.
                    return Ok(AppendEntriesReply {
                        term: self.persistent.current_term,
                        success: false,
                        conflict_index: self.last_log_index(),
                    });
                }
            }
        }

        // 4. Truncate any conflicting suffix + append.
        // Walk entries; if our log has a different term at the same index, truncate.
        let mut applied_any = false;
        for e in &args.entries {
            if let Some(existing) = self.log_at(e.index) {
                if existing.term != e.term {
                    self.truncate_log_to(e.index - 1);
                    self.append_entries_to_log(std::slice::from_ref(e));
                    applied_any = true;
                }
                // If same term + index, entries match — no-op.
            } else {
                self.append_entries_to_log(std::slice::from_ref(e));
                applied_any = true;
            }
        }
        if applied_any {
            self.save_persistent()?;
        }

        // 5. Advance commit_index.
        if args.leader_commit > self.commit_index {
            self.commit_index = args.leader_commit.min(self.last_log_index());
        }

        Ok(AppendEntriesReply {
            term: self.persistent.current_term,
            success: true,
            conflict_index: 0,
        })
    }

    /// Process a `RequestVoteReply` we asked for as a candidate.
    pub fn handle_request_vote_reply(
        &mut self,
        from: NodeId,
        reply: RequestVoteReply,
        now: Instant,
    ) -> Result<()> {
        if reply.term > self.persistent.current_term {
            self.become_follower(reply.term, None, now)?;
            return Ok(());
        }
        if self.role != Role::Candidate || reply.term < self.persistent.current_term {
            return Ok(());
        }
        if reply.vote_granted {
            self.votes_received.insert(from);
            // Promote through the ported etcd VoteResult tally (joint-aware);
            // for a single config this is identical to the strict-majority test.
            if self.election_vote_result() == cave_etcd::raft_tracker::VoteResult::Won {
                self.become_leader(now);
            }
        }
        Ok(())
    }

    /// Process an `AppendEntriesReply` we sent as leader.
    pub fn handle_append_entries_reply(
        &mut self,
        from: NodeId,
        reply: AppendEntriesReply,
        sent_prev_index: LogIndex,
        sent_entries_len: usize,
        now: Instant,
    ) -> Result<()> {
        if reply.term > self.persistent.current_term {
            self.become_follower(reply.term, None, now)?;
            return Ok(());
        }
        if self.role != Role::Leader {
            return Ok(());
        }
        if reply.success {
            let new_match = sent_prev_index + sent_entries_len as u64;
            let cur = self.match_index.get(&from).copied().unwrap_or(0);
            if new_match > cur {
                self.match_index.insert(from, new_match);
                self.next_index.insert(from, new_match + 1);
            }
            self.advance_commit_index();
        } else {
            // Backtrack. `conflict_index` is a hint; if zero, fall back one.
            let cur = self.next_index.get(&from).copied().unwrap_or(1);
            let target = if reply.conflict_index > 0 {
                reply.conflict_index.max(1).min(cur.saturating_sub(1))
            } else {
                cur.saturating_sub(1).max(1)
            };
            self.next_index.insert(from, target);
        }
        Ok(())
    }

    /// Per paper §5.4.2: leader can only commit entries from the
    /// **current term**. Walks down from `last_log_index` looking for
    /// the highest N such that a majority of `match_index[i] >= N`
    /// AND `log[N].term == current_term`.
    fn advance_commit_index(&mut self) {
        if self.role != Role::Leader {
            return;
        }
        let last = self.last_log_index();
        let current_term = self.persistent.current_term;
        let majority = self.majority();
        // Walk down.
        let mut n = last;
        while n > self.commit_index {
            // Count peers (including self) whose match >= n.
            let mut count = 1; // self
            for (_, mi) in self.match_index.iter() {
                if *mi >= n {
                    count += 1;
                }
            }
            if count >= majority {
                if let Some(e) = self.log_at(n) {
                    if e.term == current_term {
                        self.commit_index = n;
                        debug!(
                            commit_index = n,
                            term = current_term,
                            "leader committed up to N"
                        );
                        break;
                    }
                }
            }
            n -= 1;
        }
    }

    // ── Client-facing ──────────────────────────────────────────────────────

    /// Append `command` to the leader's log. Returns the assigned index.
    /// Caller polls `commit_index` (or `take_committed_entries`) to learn
    /// when it commits.
    pub fn propose(&mut self, command: Vec<u8>) -> Result<LogIndex, ProposeError> {
        if self.role != Role::Leader {
            return Err(ProposeError::NotLeader(self.role, self.leader_id));
        }
        let new_index = self.last_log_index() + 1;
        let entry = LogEntry {
            term: self.persistent.current_term,
            index: new_index,
            command,
        };
        self.persistent.log.push(entry);
        if let Err(e) = self.save_persistent() {
            // If persistence failed, we cannot honestly say the entry is
            // durable on this node. Roll back the in-memory entry.
            self.persistent.log.pop();
            warn!(error = %e, "propose: persistent save failed, rolling back");
            return Err(ProposeError::NotLeader(self.role, self.leader_id));
        }
        Ok(new_index)
    }

    /// Drain committed-but-not-yet-applied entries. The host calls this
    /// in its driver loop and applies each one to whatever state machine
    /// it cares about. Idempotent: calling twice without new commits
    /// returns the empty vec the second time.
    pub fn take_committed_entries(&mut self) -> Vec<LogEntry> {
        let mut out = Vec::new();
        while self.last_applied < self.commit_index {
            self.last_applied += 1;
            if let Some(e) = self.log_at(self.last_applied) {
                out.push(e.clone());
            }
        }
        out
    }

    // ── Tick ───────────────────────────────────────────────────────────────

    /// The host's driver task calls this on a short cadence (~50 ms).
    /// Returns any outbound RPCs the core wants sent. Idempotent if no
    /// timer has fired.
    pub fn tick(&mut self, now: Instant) -> Result<Vec<(Outbound, OutboundCtx)>> {
        let mut out = Vec::new();
        match self.role {
            Role::Follower | Role::Candidate => {
                if now >= self.election_deadline {
                    self.become_candidate(now)?;
                    let args = RequestVoteArgs {
                        term: self.persistent.current_term,
                        candidate_id: self.local_id,
                        last_log_index: self.last_log_index(),
                        last_log_term: self.last_log_term(),
                    };
                    for id in &self.members {
                        if *id == self.local_id {
                            continue;
                        }
                        out.push((
                            Outbound {
                                to: *id,
                                msg: OutboundMessage::RequestVote(args.clone()),
                            },
                            OutboundCtx::Vote,
                        ));
                    }
                }
            }
            Role::Leader => {
                if now >= self.next_heartbeat_deadline {
                    self.next_heartbeat_deadline = now + self.heartbeat_interval;
                    // For each peer, send AppendEntries from next_index.
                    let peer_ids: Vec<NodeId> = self
                        .members
                        .iter()
                        .copied()
                        .filter(|id| *id != self.local_id)
                        .collect();
                    for peer in peer_ids {
                        let next_idx = self.next_index.get(&peer).copied().unwrap_or(1);
                        let prev_log_index = next_idx.saturating_sub(1);
                        let prev_log_term = if prev_log_index == 0 {
                            0
                        } else {
                            self.log_at(prev_log_index).map(|e| e.term).unwrap_or(0)
                        };
                        let entries: Vec<LogEntry> = self
                            .persistent
                            .log
                            .iter()
                            .filter(|e| e.index >= next_idx)
                            .cloned()
                            .collect();
                        let sent_entries_len = entries.len();
                        let args = AppendEntriesArgs {
                            term: self.persistent.current_term,
                            leader_id: self.local_id,
                            prev_log_index,
                            prev_log_term,
                            entries,
                            leader_commit: self.commit_index,
                        };
                        out.push((
                            Outbound {
                                to: peer,
                                msg: OutboundMessage::AppendEntries(args),
                            },
                            OutboundCtx::Append {
                                prev_log_index,
                                entries_len: sent_entries_len,
                            },
                        ));
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Context the host needs to thread back when a reply arrives.
#[derive(Debug, Clone, Copy)]
pub enum OutboundCtx {
    Vote,
    Append {
        prev_log_index: LogIndex,
        entries_len: usize,
    },
}

/// Paper §5.4.1: candidate log is at least as up-to-date as ours iff
/// (a) its last entry's term is greater than ours, or
/// (b) its last entry's term equals ours AND its last index >= ours.
pub fn candidate_log_at_least_as_up_to_date(
    cand_last_term: Term,
    cand_last_index: LogIndex,
    my_last_term: Term,
    my_last_index: LogIndex,
) -> bool {
    cand_last_term > my_last_term
        || (cand_last_term == my_last_term && cand_last_index >= my_last_index)
}

fn derive_rng_seed(node_id: NodeId) -> u64 {
    // Mix the node id into a non-zero starting value. The xorshift step
    // doesn't progress from zero, so we OR a constant in.
    (node_id.wrapping_mul(0x9E3779B97F4A7C15)) | 0x1
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn t0() -> Instant {
        Instant::now()
    }

    fn make_core(node: NodeId, members: Vec<NodeId>, dir: &Path) -> RaftCore {
        let mut core = RaftCore::load_or_init(node, members, dir, t0()).unwrap();
        // Tighter timing for tests.
        core.set_timeout_range_ms(20, 40);
        core.set_heartbeat_interval(Duration::from_millis(5));
        core
    }

    #[test]
    fn fresh_core_starts_as_follower_at_term_zero() {
        let tmp = TempDir::new().unwrap();
        let core = make_core(1, vec![1, 2, 3], tmp.path());
        assert_eq!(core.role(), Role::Follower);
        assert_eq!(core.current_term(), 0);
        assert_eq!(core.last_log_index(), 0);
        assert_eq!(core.last_log_term(), 0);
        assert!(core.leader().is_none());
        assert!(core.voted_for().is_none());
    }

    #[test]
    fn save_persistent_roundtrips_through_disk() {
        let tmp = TempDir::new().unwrap();
        {
            let mut core = make_core(7, vec![7, 8, 9], tmp.path());
            core.persistent.current_term = 42;
            core.persistent.voted_for = Some(8);
            core.persistent.log.push(LogEntry {
                term: 42,
                index: 1,
                command: b"hello".to_vec(),
            });
            core.save_persistent().unwrap();
        }
        let core2 = RaftCore::load_or_init(7, vec![7, 8, 9], tmp.path(), t0()).unwrap();
        assert_eq!(core2.current_term(), 42);
        assert_eq!(core2.voted_for(), Some(8));
        assert_eq!(core2.last_log_index(), 1);
        assert_eq!(core2.persistent.log[0].command, b"hello".to_vec());
    }

    // ── RequestVote ─────────────────────────────────────────────────────────

    #[test]
    fn request_vote_grants_on_first_request_with_up_to_date_log() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        let reply = core
            .handle_request_vote(
                RequestVoteArgs {
                    term: 1,
                    candidate_id: 2,
                    last_log_index: 0,
                    last_log_term: 0,
                },
                t0(),
            )
            .unwrap();
        assert!(reply.vote_granted);
        assert_eq!(reply.term, 1);
        assert_eq!(core.voted_for(), Some(2));
        assert_eq!(core.current_term(), 1);
    }

    #[test]
    fn request_vote_rejects_stale_term() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        core.persistent.current_term = 5;
        core.save_persistent().unwrap();
        let reply = core
            .handle_request_vote(
                RequestVoteArgs {
                    term: 3,
                    candidate_id: 2,
                    last_log_index: 0,
                    last_log_term: 0,
                },
                t0(),
            )
            .unwrap();
        assert!(!reply.vote_granted);
        assert_eq!(reply.term, 5);
    }

    #[test]
    fn request_vote_rejects_second_candidate_in_same_term() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Vote for 2 in term 1.
        let _ = core
            .handle_request_vote(
                RequestVoteArgs {
                    term: 1,
                    candidate_id: 2,
                    last_log_index: 0,
                    last_log_term: 0,
                },
                t0(),
            )
            .unwrap();
        // Same term, different candidate → reject.
        let reply = core
            .handle_request_vote(
                RequestVoteArgs {
                    term: 1,
                    candidate_id: 3,
                    last_log_index: 0,
                    last_log_term: 0,
                },
                t0(),
            )
            .unwrap();
        assert!(!reply.vote_granted);
        assert_eq!(core.voted_for(), Some(2));
    }

    #[test]
    fn request_vote_rejects_stale_log() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Local has an entry at index=5, term=3.
        core.persistent.log = vec![
            LogEntry {
                term: 1,
                index: 1,
                command: vec![],
            },
            LogEntry {
                term: 2,
                index: 2,
                command: vec![],
            },
            LogEntry {
                term: 2,
                index: 3,
                command: vec![],
            },
            LogEntry {
                term: 3,
                index: 4,
                command: vec![],
            },
            LogEntry {
                term: 3,
                index: 5,
                command: vec![],
            },
        ];
        core.persistent.current_term = 3;
        core.save_persistent().unwrap();
        // Candidate's last entry is term=2 → behind ours.
        let reply = core
            .handle_request_vote(
                RequestVoteArgs {
                    term: 4,
                    candidate_id: 2,
                    last_log_index: 10,
                    last_log_term: 2,
                },
                t0(),
            )
            .unwrap();
        assert!(!reply.vote_granted, "must refuse candidate with stale log");
        // But the term should have been adopted.
        assert_eq!(core.current_term(), 4);
    }

    #[test]
    fn request_vote_grants_after_seeing_higher_term_even_if_previously_voted() {
        // Paper safety case: a follower that voted for X in term T must
        // still vote for Y in term T+1 if Y's log is at least as up-to-date.
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        let _ = core
            .handle_request_vote(
                RequestVoteArgs {
                    term: 5,
                    candidate_id: 2,
                    last_log_index: 0,
                    last_log_term: 0,
                },
                t0(),
            )
            .unwrap();
        assert_eq!(core.voted_for(), Some(2));
        // New election in term 6.
        let reply = core
            .handle_request_vote(
                RequestVoteArgs {
                    term: 6,
                    candidate_id: 3,
                    last_log_index: 0,
                    last_log_term: 0,
                },
                t0(),
            )
            .unwrap();
        assert!(reply.vote_granted);
        assert_eq!(core.voted_for(), Some(3));
        assert_eq!(core.current_term(), 6);
    }

    // ── AppendEntries ───────────────────────────────────────────────────────

    #[test]
    fn append_entries_rejects_stale_term() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        core.persistent.current_term = 5;
        core.save_persistent().unwrap();
        let reply = core
            .handle_append_entries(
                AppendEntriesArgs {
                    term: 3,
                    leader_id: 2,
                    prev_log_index: 0,
                    prev_log_term: 0,
                    entries: vec![],
                    leader_commit: 0,
                },
                t0(),
            )
            .unwrap();
        assert!(!reply.success);
        assert_eq!(reply.term, 5);
    }

    #[test]
    fn append_entries_appends_to_empty_log() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        let reply = core
            .handle_append_entries(
                AppendEntriesArgs {
                    term: 1,
                    leader_id: 2,
                    prev_log_index: 0,
                    prev_log_term: 0,
                    entries: vec![
                        LogEntry {
                            term: 1,
                            index: 1,
                            command: b"a".to_vec(),
                        },
                        LogEntry {
                            term: 1,
                            index: 2,
                            command: b"b".to_vec(),
                        },
                    ],
                    leader_commit: 1,
                },
                t0(),
            )
            .unwrap();
        assert!(reply.success);
        assert_eq!(core.last_log_index(), 2);
        assert_eq!(core.commit_index(), 1);
        assert_eq!(core.leader(), Some(2));
    }

    #[test]
    fn append_entries_rejects_on_prev_log_mismatch() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Local has [term=1, index=1].
        core.persistent.log.push(LogEntry {
            term: 1,
            index: 1,
            command: vec![],
        });
        core.persistent.current_term = 2;
        core.save_persistent().unwrap();
        // Leader asks for prev_log_index=1, prev_log_term=2 — mismatch.
        let reply = core
            .handle_append_entries(
                AppendEntriesArgs {
                    term: 2,
                    leader_id: 2,
                    prev_log_index: 1,
                    prev_log_term: 2,
                    entries: vec![LogEntry {
                        term: 2,
                        index: 2,
                        command: vec![],
                    }],
                    leader_commit: 0,
                },
                t0(),
            )
            .unwrap();
        assert!(!reply.success);
        assert!(reply.conflict_index <= 1);
    }

    #[test]
    fn append_entries_truncates_conflicting_suffix() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Local has 3 entries, the last is in a different term than leader has.
        core.persistent.log = vec![
            LogEntry {
                term: 1,
                index: 1,
                command: vec![],
            },
            LogEntry {
                term: 1,
                index: 2,
                command: vec![],
            },
            LogEntry {
                term: 3,
                index: 3,
                command: b"old".to_vec(),
            },
        ];
        core.persistent.current_term = 5;
        core.save_persistent().unwrap();
        // Leader sends entry index=3 with term=5, prev=2/1.
        let reply = core
            .handle_append_entries(
                AppendEntriesArgs {
                    term: 5,
                    leader_id: 2,
                    prev_log_index: 2,
                    prev_log_term: 1,
                    entries: vec![LogEntry {
                        term: 5,
                        index: 3,
                        command: b"new".to_vec(),
                    }],
                    leader_commit: 0,
                },
                t0(),
            )
            .unwrap();
        assert!(reply.success);
        assert_eq!(core.persistent.log.len(), 3);
        assert_eq!(core.persistent.log[2].term, 5);
        assert_eq!(core.persistent.log[2].command, b"new".to_vec());
    }

    #[test]
    fn append_entries_steps_down_candidate_on_equal_or_higher_term() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        core.become_candidate(t0()).unwrap();
        assert_eq!(core.role(), Role::Candidate);
        let term = core.current_term();
        let _ = core
            .handle_append_entries(
                AppendEntriesArgs {
                    term,
                    leader_id: 2,
                    prev_log_index: 0,
                    prev_log_term: 0,
                    entries: vec![],
                    leader_commit: 0,
                },
                t0(),
            )
            .unwrap();
        assert_eq!(core.role(), Role::Follower);
        assert_eq!(core.leader(), Some(2));
    }

    // ── Election ────────────────────────────────────────────────────────────

    #[test]
    fn election_tick_promotes_to_candidate_after_timeout() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        let start = t0();
        // Force the election deadline into the past.
        let future = start + Duration::from_millis(500);
        let out = core.tick(future).unwrap();
        assert_eq!(core.role(), Role::Candidate);
        assert_eq!(core.current_term(), 1);
        assert_eq!(core.voted_for(), Some(1)); // voted for self
        // Outbound RequestVote to 2 peers.
        assert_eq!(out.len(), 2);
        for (ob, _ctx) in &out {
            match &ob.msg {
                OutboundMessage::RequestVote(args) => {
                    assert_eq!(args.candidate_id, 1);
                    assert_eq!(args.term, 1);
                }
                _ => panic!("expected RequestVote"),
            }
        }
    }

    #[test]
    fn candidate_with_majority_votes_becomes_leader() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Become candidate.
        core.become_candidate(t0()).unwrap();
        assert_eq!(core.role(), Role::Candidate);
        // One peer grants — combined with self-vote that's 2/3 = majority.
        core.handle_request_vote_reply(
            2,
            RequestVoteReply {
                term: 1,
                vote_granted: true,
            },
            t0(),
        )
        .unwrap();
        assert_eq!(core.role(), Role::Leader);
        assert_eq!(core.leader(), Some(1));
    }

    #[test]
    fn election_vote_result_routes_through_ported_tracker() {
        use cave_etcd::raft_tracker::VoteResult;
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Before campaigning: no self-vote recorded → not yet decided.
        assert_eq!(core.election_vote_result(), VoteResult::Pending);
        // Candidate records its own vote (1/3) → still Pending.
        core.become_candidate(t0()).unwrap();
        assert_eq!(core.election_vote_result(), VoteResult::Pending);
        // One peer grant → 2/3 majority → Won, and the reply path must promote
        // the node to Leader using exactly this outcome.
        core.handle_request_vote_reply(
            2,
            RequestVoteReply {
                term: 1,
                vote_granted: true,
            },
            t0(),
        )
        .unwrap();
        assert_eq!(core.election_vote_result(), VoteResult::Won);
        assert_eq!(core.role(), Role::Leader);
    }

    #[test]
    fn candidate_without_majority_stays_candidate() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3, 4, 5], tmp.path());
        core.become_candidate(t0()).unwrap();
        // One peer grants — self + one = 2/5, not majority (3).
        core.handle_request_vote_reply(
            2,
            RequestVoteReply {
                term: 1,
                vote_granted: true,
            },
            t0(),
        )
        .unwrap();
        assert_eq!(core.role(), Role::Candidate);
    }

    #[test]
    fn higher_term_reply_steps_candidate_down() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        core.become_candidate(t0()).unwrap();
        core.handle_request_vote_reply(
            2,
            RequestVoteReply {
                term: 99,
                vote_granted: false,
            },
            t0(),
        )
        .unwrap();
        assert_eq!(core.role(), Role::Follower);
        assert_eq!(core.current_term(), 99);
    }

    // ── Leader log + commit ─────────────────────────────────────────────────

    #[test]
    fn propose_on_follower_returns_not_leader() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        let err = core.propose(b"x".to_vec()).unwrap_err();
        assert!(matches!(err, ProposeError::NotLeader(Role::Follower, _)));
    }

    #[test]
    fn propose_on_leader_appends_to_log() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        core.become_candidate(t0()).unwrap();
        core.become_leader(t0());
        let idx = core.propose(b"hello".to_vec()).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(core.last_log_index(), 1);
        assert_eq!(core.persistent.log[0].command, b"hello".to_vec());
    }

    #[test]
    fn leader_advances_commit_on_majority_match() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        core.become_candidate(t0()).unwrap();
        core.become_leader(t0());
        core.propose(b"a".to_vec()).unwrap();
        core.propose(b"b".to_vec()).unwrap();
        // Peer 2 caught up to index 2. With self counting, that's 2/3 = majority.
        core.handle_append_entries_reply(
            2,
            AppendEntriesReply {
                term: core.current_term(),
                success: true,
                conflict_index: 0,
            },
            /* sent_prev_index */ 0,
            /* sent_entries_len */ 2,
            t0(),
        )
        .unwrap();
        assert_eq!(core.commit_index(), 2);
    }

    #[test]
    fn leader_only_commits_entries_in_current_term() {
        // Paper §5.4.2 figure 8: an entry from a previous term must not
        // be committed by counting replicas alone; it commits indirectly
        // when the leader replicates a later entry from its own term.
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Synthesise a previous-term entry directly.
        core.persistent.log.push(LogEntry {
            term: 2,
            index: 1,
            command: b"old".to_vec(),
        });
        core.persistent.current_term = 5;
        core.save_persistent().unwrap();
        core.become_candidate(t0()).unwrap(); // term -> 6
        core.become_leader(t0());

        // Peer 2 matches up to index 1 (the old entry). Majority match
        // exists but the entry is NOT from the current term — must not commit.
        core.handle_append_entries_reply(
            2,
            AppendEntriesReply {
                term: core.current_term(),
                success: true,
                conflict_index: 0,
            },
            0,
            1,
            t0(),
        )
        .unwrap();
        assert_eq!(core.commit_index(), 0, "must not commit old-term entry");

        // Now propose a current-term entry and ack it.
        core.propose(b"new".to_vec()).unwrap();
        core.handle_append_entries_reply(
            2,
            AppendEntriesReply {
                term: core.current_term(),
                success: true,
                conflict_index: 0,
            },
            0,
            2,
            t0(),
        )
        .unwrap();
        // Both entries now committed.
        assert_eq!(core.commit_index(), 2);
    }

    #[test]
    fn append_reply_failure_backtracks_next_index() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        // Set up: leader at term 1, log has 5 entries, peer 2's next_index = 6.
        core.become_candidate(t0()).unwrap();
        core.become_leader(t0());
        // Push 5 entries and manually push next_index to mirror what the
        // leader would do after a successful round.
        for _ in 0..5 {
            core.propose(b"x".to_vec()).unwrap();
        }
        core.next_index.insert(2, 6);

        // Now a failure response should backtrack.
        core.handle_append_entries_reply(
            2,
            AppendEntriesReply {
                term: core.current_term(),
                success: false,
                conflict_index: 0,
            },
            /* sent_prev_index */ 5,
            /* sent_entries_len */ 0,
            t0(),
        )
        .unwrap();
        assert_eq!(core.next_index.get(&2).copied(), Some(5));

        // Another failure with conflict_index = 2 should jump back to 2.
        core.handle_append_entries_reply(
            2,
            AppendEntriesReply {
                term: core.current_term(),
                success: false,
                conflict_index: 2,
            },
            4,
            0,
            t0(),
        )
        .unwrap();
        assert_eq!(core.next_index.get(&2).copied(), Some(2));
    }

    #[test]
    fn take_committed_entries_drains_in_order() {
        let tmp = TempDir::new().unwrap();
        let mut core = make_core(1, vec![1, 2, 3], tmp.path());
        core.become_candidate(t0()).unwrap();
        core.become_leader(t0());
        core.propose(b"a".to_vec()).unwrap();
        core.propose(b"b".to_vec()).unwrap();
        core.propose(b"c".to_vec()).unwrap();
        // Ack from peer 2 → 3 entries replicated → commit_index = 3.
        core.handle_append_entries_reply(
            2,
            AppendEntriesReply {
                term: core.current_term(),
                success: true,
                conflict_index: 0,
            },
            0,
            3,
            t0(),
        )
        .unwrap();
        let drained = core.take_committed_entries();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].command, b"a".to_vec());
        assert_eq!(drained[2].command, b"c".to_vec());
        // Second drain returns empty.
        assert!(core.take_committed_entries().is_empty());
    }

    // ── Helpers ─────────────────────────────────────────────────────────────

    #[test]
    fn candidate_log_check_paper_table_examples() {
        // (cand_term, cand_idx, my_term, my_idx, expected)
        let cases = [
            (1, 1, 1, 1, true),  // equal
            (2, 1, 1, 5, true),  // higher term beats higher index
            (1, 5, 1, 1, true),  // same term, candidate further ahead
            (1, 1, 1, 5, false), // same term, candidate behind
            (1, 1, 2, 1, false), // candidate term behind
        ];
        for (ct, ci, mt, mi, expected) in cases {
            assert_eq!(
                candidate_log_at_least_as_up_to_date(ct, ci, mt, mi),
                expected,
                "case ({}, {}, {}, {})",
                ct,
                ci,
                mt,
                mi
            );
        }
    }

    // ── Integration: 3 cores piped through manual delivery ──────────────────

    /// Locate `cores` by node_id and return its position; returns None
    /// when the node was "killed" (removed from the vec) in a failover
    /// test. The harness silently drops messages addressed to dead nodes.
    fn pos_of(cores: &[RaftCore], node_id: NodeId) -> Option<usize> {
        cores.iter().position(|c| c.local_id == node_id)
    }

    /// One driver step: tick every core, collect outbounds, deliver them
    /// to receivers (collecting replies), then ship replies back to senders.
    /// Messages addressed to absent nodes are silently dropped (simulates
    /// a partition / dead peer).
    fn drive_step(cores: &mut Vec<RaftCore>, now: Instant) {
        // Phase 1: tick each core, capture (from, ob, ctx).
        let mut outbounds: Vec<(NodeId, Outbound, OutboundCtx)> = Vec::new();
        for core in cores.iter_mut() {
            let from = core.local_id;
            for (ob, ctx) in core.tick(now).unwrap() {
                outbounds.push((from, ob, ctx));
            }
        }
        // Phase 2: deliver to receivers we still have; capture replies.
        let mut replies: Vec<(NodeId, NodeId, ReplyKind)> = Vec::new();
        for (from, ob, ctx) in outbounds {
            let to = ob.to;
            let recv_idx = match pos_of(cores, to) {
                Some(i) => i,
                None => continue, // dead node, drop the message
            };
            match (ob.msg, ctx) {
                (OutboundMessage::RequestVote(args), OutboundCtx::Vote) => {
                    let reply = cores[recv_idx].handle_request_vote(args, now).unwrap();
                    replies.push((from, to, ReplyKind::Vote(reply)));
                }
                (
                    OutboundMessage::AppendEntries(args),
                    OutboundCtx::Append {
                        prev_log_index,
                        entries_len,
                    },
                ) => {
                    let reply = cores[recv_idx].handle_append_entries(args, now).unwrap();
                    replies.push((
                        from,
                        to,
                        ReplyKind::Append {
                            reply,
                            prev_log_index,
                            entries_len,
                        },
                    ));
                }
                _ => {}
            }
        }
        // Phase 3: ship replies back. Sender may also be dead (in a
        // partition test), so check.
        for (sender_id, target_id, kind) in replies {
            let sender_idx = match pos_of(cores, sender_id) {
                Some(i) => i,
                None => continue,
            };
            match kind {
                ReplyKind::Vote(r) => {
                    cores[sender_idx]
                        .handle_request_vote_reply(target_id, r, now)
                        .unwrap();
                }
                ReplyKind::Append {
                    reply,
                    prev_log_index,
                    entries_len,
                } => {
                    cores[sender_idx]
                        .handle_append_entries_reply(
                            target_id,
                            reply,
                            prev_log_index,
                            entries_len,
                            now,
                        )
                        .unwrap();
                }
            }
        }
    }

    enum ReplyKind {
        Vote(RequestVoteReply),
        Append {
            reply: AppendEntriesReply,
            prev_log_index: LogIndex,
            entries_len: usize,
        },
    }

    fn three_cores(tmp: &TempDir) -> Vec<RaftCore> {
        let mut out = Vec::new();
        for n in 1..=3u64 {
            let d = tmp.path().join(format!("n{n}"));
            std::fs::create_dir_all(&d).unwrap();
            out.push(make_core(n, vec![1, 2, 3], &d));
        }
        out
    }

    #[test]
    fn three_cores_elect_a_leader_with_in_process_transport() {
        let tmp = TempDir::new().unwrap();
        let mut cores = three_cores(&tmp);

        let mut now = t0();
        let mut leader: Option<NodeId> = None;
        for _ in 0..200 {
            drive_step(&mut cores, now);
            if let Some(c) = cores.iter().find(|c| c.role() == Role::Leader) {
                leader = Some(c.local_id);
                break;
            }
            now += Duration::from_millis(10);
        }
        assert!(leader.is_some(), "no leader elected after 200 ticks");
        let leader_term = cores
            .iter()
            .find(|c| c.role() == Role::Leader)
            .map(|c| c.current_term())
            .unwrap();
        for c in &cores {
            assert!(
                c.current_term() >= leader_term.saturating_sub(1),
                "node {} term {} too far from leader's {}",
                c.local_id,
                c.current_term(),
                leader_term
            );
        }
    }

    #[test]
    fn three_cores_replicate_a_proposed_entry() {
        let tmp = TempDir::new().unwrap();
        let mut cores = three_cores(&tmp);

        let mut now = t0();
        let mut proposed = false;
        for _ in 0..400 {
            drive_step(&mut cores, now);

            // Once a leader exists, propose a command.
            if !proposed {
                if let Some(leader_id) = cores
                    .iter()
                    .find(|c| c.role() == Role::Leader)
                    .map(|c| c.local_id)
                {
                    let idx = pos_of(&cores, leader_id).unwrap();
                    cores[idx].propose(b"hello-cluster".to_vec()).unwrap();
                    proposed = true;
                }
            }

            // Done when every node has the entry AND the leader has committed.
            if proposed {
                let leader = cores.iter().find(|c| c.role() == Role::Leader);
                if let Some(l) = leader {
                    let leader_id = l.local_id;
                    let leader_commit = l.commit_index();
                    let all_have_entry = cores.iter().all(|c| c.last_log_index() >= 1);
                    if leader_commit >= 1 && all_have_entry {
                        let leader_idx = pos_of(&cores, leader_id).unwrap();
                        let drained = cores[leader_idx].take_committed_entries();
                        assert_eq!(drained.len(), 1);
                        assert_eq!(drained[0].command, b"hello-cluster".to_vec());
                        return;
                    }
                }
            }
            now += Duration::from_millis(10);
        }
        panic!(
            "no replicated commit after 400 ticks (proposed={}, log_indices={:?})",
            proposed,
            cores.iter().map(|c| c.last_log_index()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn killed_leader_failover_elects_new_leader_and_replicates() {
        // 1. Three cores form a cluster, leader emerges.
        // 2. "Kill" the leader by removing it from `cores`.
        // 3. Drive ticks; remaining two should elect a new leader.
        // 4. Propose on the new leader; verify it replicates to the surviving peer.
        let tmp = TempDir::new().unwrap();
        let mut cores = three_cores(&tmp);

        let mut now = t0();
        let original_leader = loop {
            drive_step(&mut cores, now);
            if let Some(c) = cores.iter().find(|c| c.role() == Role::Leader) {
                break c.local_id;
            }
            now += Duration::from_millis(10);
            if now.duration_since(t0()) > Duration::from_secs(5) {
                panic!("no initial leader after 5s of simulated time");
            }
        };

        // Drop the leader.
        cores.retain(|c| c.local_id != original_leader);
        assert_eq!(cores.len(), 2);

        // Drive failover. Election with 2/3 alive: majority = 2, so the
        // surviving pair can still elect.
        let mut new_leader: Option<NodeId> = None;
        for _ in 0..500 {
            drive_step(&mut cores, now);
            if let Some(c) = cores.iter().find(|c| c.role() == Role::Leader) {
                new_leader = Some(c.local_id);
                break;
            }
            now += Duration::from_millis(10);
        }
        let new_leader = new_leader.expect("surviving pair must elect");
        assert_ne!(new_leader, original_leader);

        // Propose on the new leader and verify replication to the other surviving peer.
        let idx = pos_of(&cores, new_leader).unwrap();
        cores[idx].propose(b"post-failover".to_vec()).unwrap();
        let mut replicated = false;
        for _ in 0..200 {
            drive_step(&mut cores, now);
            let leader_commit = cores
                .iter()
                .find(|c| c.local_id == new_leader)
                .map(|c| c.commit_index())
                .unwrap_or(0);
            if leader_commit >= 1 && cores.iter().all(|c| c.last_log_index() >= 1) {
                replicated = true;
                break;
            }
            now += Duration::from_millis(10);
        }
        assert!(replicated, "new leader must replicate to surviving peer");
    }
}

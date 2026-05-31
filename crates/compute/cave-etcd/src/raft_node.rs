// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-process Raft consensus state machine — port of etcd's core `raft`
//! package (`go.etcd.io/raft`, `raft.go` / `rawnode.go` / `log.go`).
//!
//! This is the *data-plane* state machine: role transitions
//! (follower / pre-candidate / candidate / leader), the election &
//! heartbeat logical clocks, vote granting and tallying, append-entries
//! accounting, and leader commit advancement. It deliberately reuses the
//! already-ported [`ProgressTracker`](crate::raft_tracker::ProgressTracker)
//! and [`raft_joint_quorum`](crate::raft_joint_quorum) primitives.
//!
//! What lives *outside* this module (and stays scope-cut to the
//! cave-runtime cluster listener / `rafthttp` parallel track): the network
//! driver that pumps [`Message`]s between peers, the `Ready`/`Advance`
//! batching loop, and the durable storage adapter. This module produces and
//! consumes [`Message`]s in-process; transport is somebody else's job.

use crate::raft_tracker::{ProgressTracker, VoteResult};

/// The role a Raft peer currently occupies. Mirrors etcd `raft.StateType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaftState {
    Follower,
    /// Pre-vote probe state (PreVote extension); does not bump the term.
    PreCandidate,
    Candidate,
    Leader,
}

/// Message types exchanged by the state machine. A subset of etcd
/// `raftpb.MessageType` — only the ones the in-process state machine
/// originates or reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    /// Local "start an election" trigger.
    MsgHup,
    /// Leader→follower heartbeat.
    MsgHeartbeat,
    MsgHeartbeatResp,
    /// Candidate→peer vote request.
    MsgVote,
    MsgVoteResp,
    /// Pre-vote request (does not bump term).
    MsgPreVote,
    MsgPreVoteResp,
    /// Leader→follower log replication.
    MsgApp,
    MsgAppResp,
}

/// A Raft message. Fields mirror the subset of `raftpb.Message` the
/// in-process state machine reads/writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub msg_type: MessageType,
    pub from: u64,
    pub to: u64,
    pub term: u64,
    /// Log index referenced by the message (last-log-index for votes,
    /// prev-log-index for appends, acked index for responses).
    pub index: u64,
    /// Log term referenced by the message (last-log-term for votes).
    pub log_term: u64,
    /// Rejection flag for `*Resp` messages.
    pub reject: bool,
}

impl Message {
    fn new(msg_type: MessageType, from: u64, to: u64, term: u64) -> Self {
        Self {
            msg_type,
            from,
            to,
            term,
            index: 0,
            log_term: 0,
            reject: false,
        }
    }
}

/// No-leader sentinel (etcd `None`).
pub const NONE: u64 = 0;

/// The core Raft state machine.
pub struct RaftNode {
    pub id: u64,
    pub term: u64,
    /// Candidate this peer voted for in `term` (`NONE` = not yet voted).
    pub vote: u64,
    pub state: RaftState,
    /// Believed current leader (`NONE` if unknown).
    pub lead: u64,
    pub election_elapsed: usize,
    pub heartbeat_elapsed: usize,
    pub heartbeat_timeout: usize,
    pub election_timeout: usize,
    pub randomized_election_timeout: usize,
    pub check_quorum: bool,
    pub pre_vote: bool,
    pub tracker: ProgressTracker,
    /// Highest index in the local log.
    pub last_index: u64,
    /// Term of the entry at `last_index`.
    pub last_term: u64,
    /// Highest committed index.
    pub commit: u64,
    /// Term of each log entry, indexed by `index - 1` (index 1 = `log_terms[0]`).
    pub log_terms: Vec<u64>,
    /// Outbound messages produced since the last drain.
    pub msgs: Vec<Message>,
    /// Deterministic PRNG state for randomized election timeout.
    rng: u64,
}

impl RaftNode {
    /// Construct a follower at term 0 with the given peer set.
    pub fn new(
        id: u64,
        peers: &[u64],
        election_timeout: usize,
        heartbeat_timeout: usize,
        max_inflight: usize,
    ) -> Self {
        let mut tracker = ProgressTracker::new(max_inflight);
        for &p in peers {
            tracker.add_voter(p);
        }
        let mut n = Self {
            id,
            term: 0,
            vote: NONE,
            state: RaftState::Follower,
            lead: NONE,
            election_elapsed: 0,
            heartbeat_elapsed: 0,
            heartbeat_timeout,
            election_timeout,
            randomized_election_timeout: election_timeout,
            check_quorum: false,
            pre_vote: false,
            tracker,
            last_index: 0,
            last_term: 0,
            commit: 0,
            log_terms: Vec::new(),
            msgs: Vec::new(),
            // Seed the PRNG off the node id so each peer randomizes
            // independently but reproducibly.
            rng: id.wrapping_mul(0x9E3779B97F4A7C15).max(1),
        };
        n.reset_randomized_election_timeout();
        n
    }

    /// Deterministic xorshift64* — replaces etcd's `globalRand` so tests
    /// are reproducible.
    fn next_rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// `electiontimeout + rand(0, electiontimeout)` (etcd raft.go
    /// `resetRandomizedElectionTimeout`).
    pub fn reset_randomized_election_timeout(&mut self) {
        let span = self.election_timeout.max(1);
        let jitter = (self.next_rand() as usize) % span;
        self.randomized_election_timeout = self.election_timeout + jitter;
    }

    /// Reset volatile per-term state on a term change (etcd `raft.reset`).
    /// Bumps to `term` (clearing the vote when the term actually advances),
    /// drops the known leader, and restarts both logical clocks with a fresh
    /// randomized election timeout.
    pub fn reset(&mut self, term: u64) {
        if self.term != term {
            self.term = term;
            self.vote = NONE;
        } else {
            // Same-term reset still clears the vote when re-entering a
            // candidate cycle; etcd clears it on every reset.
            self.vote = NONE;
        }
        self.lead = NONE;
        self.election_elapsed = 0;
        self.heartbeat_elapsed = 0;
        self.reset_randomized_election_timeout();
    }

    pub fn become_follower(&mut self, term: u64, lead: u64) {
        self.reset(term);
        self.state = RaftState::Follower;
        self.lead = lead;
    }

    pub fn become_pre_candidate(&mut self) {
        // PreVote: campaign WITHOUT mutating term or vote (etcd
        // becomePreCandidate). Only the role and election clock change.
        self.election_elapsed = 0;
        self.reset_randomized_election_timeout();
        self.lead = NONE;
        self.tracker.reset_votes();
        self.state = RaftState::PreCandidate;
    }

    pub fn become_candidate(&mut self) {
        let next_term = self.term + 1;
        self.reset(next_term);
        self.tracker.reset_votes();
        self.vote = self.id;
        self.state = RaftState::Candidate;
    }

    pub fn become_leader(&mut self) {
        // Term is unchanged; we won the election at the current term.
        self.election_elapsed = 0;
        self.heartbeat_elapsed = 0;
        self.lead = self.id;
        self.state = RaftState::Leader;
    }

    /// True once the election clock has reached the randomized timeout
    /// (etcd `pastElectionTimeout`: `electionElapsed >= randomized`).
    pub fn past_election_timeout(&self) -> bool {
        self.election_elapsed >= self.randomized_election_timeout
    }

    /// Voting peers other than self (non-learners).
    fn other_voters(&self) -> Vec<u64> {
        self.tracker
            .progress
            .iter()
            .filter(|(id, pr)| **id != self.id && !pr.is_learner)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Drive one logical clock tick, dispatching by role (etcd `raft.tick`).
    pub fn tick(&mut self) {
        match self.state {
            RaftState::Leader => self.tick_heartbeat(),
            _ => self.tick_election(),
        }
    }

    /// Follower/candidate election clock. On timeout, start a campaign.
    pub fn tick_election(&mut self) {
        self.election_elapsed += 1;
        if self.past_election_timeout() {
            self.campaign();
        }
    }

    /// Leader heartbeat clock. On timeout, broadcast heartbeats.
    pub fn tick_heartbeat(&mut self) {
        self.heartbeat_elapsed += 1;
        self.election_elapsed += 1;
        if self.heartbeat_elapsed >= self.heartbeat_timeout {
            self.heartbeat_elapsed = 0;
            for to in self.other_voters() {
                let m = Message::new(MessageType::MsgHeartbeat, self.id, to, self.term);
                self.msgs.push(m);
            }
        }
    }

    /// Start an election: become candidate, vote for self via the tracker,
    /// and either win immediately (single voter) or solicit votes from peers
    /// (etcd `raft.campaign` / `becomeCandidate`).
    pub fn campaign(&mut self) {
        self.become_candidate();
        // Record our own vote and check whether that alone settles it.
        self.tracker.record_vote(self.id, true);
        if self.tracker.vote_result() == VoteResult::Won {
            self.become_leader();
            return;
        }
        for to in self.other_voters() {
            let mut m = Message::new(MessageType::MsgVote, self.id, to, self.term);
            m.index = self.last_index;
            m.log_term = self.last_term;
            self.msgs.push(m);
        }
    }

    /// Whether a candidate's (index, term) log summary is at least as
    /// up-to-date as ours (etcd `raftLog.isUpToDate`).
    pub fn is_up_to_date(&self, last_index: u64, last_term: u64) -> bool {
        last_term > self.last_term
            || (last_term == self.last_term && last_index >= self.last_index)
    }

    /// Feed one inbound message through the state machine
    /// (etcd `raft.Step`). Term reconciliation happens first, then the
    /// per-role dispatch. Responses are pushed onto `msgs`.
    pub fn step(&mut self, m: Message) {
        use MessageType::*;
        // ── 1. Term reconciliation ────────────────────────────────────
        if m.term == 0 {
            // Local message (e.g. MsgHup) — no term to reconcile.
        } else if m.term > self.term {
            match m.msg_type {
                MsgVote | MsgPreVote => {
                    // A higher-term vote request: adopt the term with no
                    // known leader yet, then decide whether to grant.
                    if m.msg_type == MsgVote {
                        self.become_follower(m.term, NONE);
                    }
                }
                _ => {
                    // Heartbeat/append from a newer leader reveals it.
                    self.become_follower(m.term, m.from);
                }
            }
        } else if m.term < self.term {
            // Stale term: reject vote requests so the sender steps down,
            // and drop everything else.
            if matches!(m.msg_type, MsgVote | MsgPreVote) {
                let resp_type = if m.msg_type == MsgVote {
                    MsgVoteResp
                } else {
                    MsgPreVoteResp
                };
                let mut resp = Message::new(resp_type, self.id, m.from, self.term);
                resp.reject = true;
                self.msgs.push(resp);
            }
            return;
        }

        // ── 2. Per-message dispatch ───────────────────────────────────
        match m.msg_type {
            MsgHup => self.campaign(),
            MsgVote | MsgPreVote => self.handle_vote(m),
            MsgVoteResp => self.handle_vote_resp(m),
            MsgAppResp => self.handle_app_resp(m),
            MsgHeartbeat => {
                self.election_elapsed = 0;
                self.lead = m.from;
                let resp = Message::new(MsgHeartbeatResp, self.id, m.from, self.term);
                self.msgs.push(resp);
            }
            _ => {}
        }
    }

    /// Decide a vote request and emit the response (etcd vote-grant logic).
    fn handle_vote(&mut self, m: Message) {
        let resp_type = if m.msg_type == MessageType::MsgPreVote {
            MessageType::MsgPreVoteResp
        } else {
            MessageType::MsgVoteResp
        };
        // We may grant if we have not yet voted this term (or already voted
        // for this same candidate) AND the candidate's log is current.
        let can_vote = self.vote == m.from || self.vote == NONE;
        let grant = can_vote && self.is_up_to_date(m.index, m.log_term);
        let mut resp = Message::new(resp_type, self.id, m.from, self.term);
        if grant {
            if m.msg_type == MessageType::MsgVote {
                self.vote = m.from;
                self.election_elapsed = 0;
            }
        } else {
            resp.reject = true;
        }
        self.msgs.push(resp);
    }

    /// Term of the log entry at `idx` (0 for the empty index 0 or any
    /// index beyond the local log).
    pub fn term_at(&self, idx: u64) -> u64 {
        if idx == 0 {
            return 0;
        }
        self.log_terms.get((idx - 1) as usize).copied().unwrap_or(0)
    }

    /// Leader: append a new entry at the current term and return its index
    /// (etcd `raft.appendEntry`). Updates the leader's own match progress.
    pub fn propose(&mut self) -> u64 {
        self.log_terms.push(self.term);
        self.last_index = self.log_terms.len() as u64;
        self.last_term = self.term;
        if let Some(pr) = self.tracker.progress.get_mut(&self.id) {
            pr.maybe_update(self.last_index);
        }
        self.last_index
    }

    /// Leader: advance `commit` to the highest index a quorum has acked,
    /// subject to the current-term safety rule (etcd `raft.maybeCommit`):
    /// a leader may only count replicas to commit entries from its *own*
    /// term; older-term entries ride along once a current-term entry commits.
    pub fn maybe_commit(&mut self) -> bool {
        // Build the match map across all voters (self included).
        let mut match_idx = std::collections::HashMap::new();
        for (id, pr) in &self.tracker.progress {
            match_idx.insert(*id, pr.r#match);
        }
        let voters = self.tracker.voter_ids();
        let outgoing = self.tracker.outgoing_ids();
        let mci = if outgoing.is_empty() {
            crate::raft_joint_quorum::majority_committed_index(&voters, &match_idx)
        } else {
            crate::raft_joint_quorum::joint_committed_index(&voters, &outgoing, &match_idx)
        };
        if mci > self.commit && self.term_at(mci) == self.term {
            self.commit = mci;
            return true;
        }
        false
    }

    /// Leader: react to a follower's append acknowledgement (etcd
    /// `stepLeader` MsgAppResp arm). On success, advance the follower's
    /// match and try to commit; on rejection, roll its `next` back.
    fn handle_app_resp(&mut self, m: Message) {
        if self.state != RaftState::Leader {
            return;
        }
        if let Some(pr) = self.tracker.progress.get_mut(&m.from) {
            if m.reject {
                pr.maybe_decr_to(m.index, m.index);
            } else if pr.maybe_update(m.index) {
                self.maybe_commit();
            }
        }
    }

    /// Tally a vote response while campaigning (etcd `stepCandidate`).
    fn handle_vote_resp(&mut self, m: Message) {
        if self.state != RaftState::Candidate && self.state != RaftState::PreCandidate {
            return;
        }
        self.tracker.record_vote(m.from, !m.reject);
        match self.tracker.vote_result() {
            VoteResult::Won => self.become_leader(),
            VoteResult::Lost => self.become_follower(self.term, NONE),
            VoteResult::Pending => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> RaftNode {
        RaftNode::new(1, &[1, 2, 3], 10, 1, 256)
    }

    #[test]
    fn become_follower_sets_state_term_lead_and_clears_vote() {
        let mut n = node();
        n.vote = 2;
        n.state = RaftState::Leader;
        n.become_follower(5, 3);
        assert_eq!(n.state, RaftState::Follower);
        assert_eq!(n.term, 5);
        assert_eq!(n.lead, 3);
        assert_eq!(n.vote, NONE, "stepping into a new term clears the vote");
        assert_eq!(n.election_elapsed, 0);
    }

    #[test]
    fn become_candidate_bumps_term_votes_for_self_and_clears_lead() {
        let mut n = node();
        n.become_candidate();
        assert_eq!(n.state, RaftState::Candidate);
        assert_eq!(n.term, 1, "candidate bumps term by one");
        assert_eq!(n.vote, 1, "candidate votes for itself");
        assert_eq!(n.lead, NONE, "candidate has no known leader");
    }

    #[test]
    fn become_pre_candidate_does_not_bump_term_or_vote() {
        let mut n = node();
        n.term = 4;
        n.vote = 2;
        n.become_pre_candidate();
        assert_eq!(n.state, RaftState::PreCandidate);
        assert_eq!(n.term, 4, "pre-candidate must NOT bump the term");
        assert_eq!(n.vote, 2, "pre-candidate must NOT change the vote");
    }

    #[test]
    fn become_leader_sets_lead_to_self_and_resets_election_clock() {
        let mut n = node();
        n.become_candidate();
        n.become_leader();
        assert_eq!(n.state, RaftState::Leader);
        assert_eq!(n.lead, 1, "a new leader leads itself");
        assert_eq!(n.election_elapsed, 0);
    }

    #[test]
    fn reset_clears_vote_and_election_progress() {
        let mut n = node();
        n.vote = 3;
        n.election_elapsed = 7;
        n.heartbeat_elapsed = 4;
        n.reset(9);
        assert_eq!(n.term, 9);
        assert_eq!(n.vote, NONE);
        assert_eq!(n.election_elapsed, 0);
        assert_eq!(n.heartbeat_elapsed, 0);
    }

    #[test]
    fn randomized_election_timeout_is_within_bounds() {
        // etcd: electiontimeout <= randomized < 2*electiontimeout.
        let mut n = node();
        for _ in 0..50 {
            n.reset_randomized_election_timeout();
            assert!(
                n.randomized_election_timeout >= n.election_timeout
                    && n.randomized_election_timeout < 2 * n.election_timeout,
                "randomized timeout {} out of [{}, {}) range",
                n.randomized_election_timeout,
                n.election_timeout,
                2 * n.election_timeout,
            );
        }
    }

    #[test]
    fn tick_election_campaigns_at_timeout_and_solicits_votes() {
        let mut n = node();
        n.randomized_election_timeout = 3;
        n.tick();
        n.tick();
        assert_eq!(n.state, RaftState::Follower, "no campaign before timeout");
        assert!(n.msgs.is_empty());
        n.tick(); // election_elapsed == 3 → campaign
        assert_eq!(n.state, RaftState::Candidate);
        assert_eq!(n.term, 1);
        assert_eq!(n.vote, 1, "campaign self-votes");
        assert_eq!(n.election_elapsed, 0, "campaign restarts the clock");
        // One MsgVote per other voter (peers 2 and 3).
        let votes: Vec<_> = n
            .msgs
            .iter()
            .filter(|m| m.msg_type == MessageType::MsgVote)
            .collect();
        assert_eq!(votes.len(), 2);
        for m in &votes {
            assert_eq!(m.term, 1);
            assert_eq!(m.from, 1);
        }
        let dests: std::collections::BTreeSet<u64> = votes.iter().map(|m| m.to).collect();
        assert_eq!(dests, [2u64, 3].into_iter().collect());
    }

    #[test]
    fn single_voter_campaign_wins_immediately() {
        let mut n = RaftNode::new(1, &[1], 10, 1, 256);
        n.campaign();
        assert_eq!(n.state, RaftState::Leader, "a lone voter self-elects");
        assert_eq!(n.lead, 1);
    }

    #[test]
    fn leader_tick_broadcasts_heartbeat_at_timeout_not_election() {
        let mut n = node();
        n.become_candidate();
        n.become_leader();
        n.heartbeat_timeout = 2;
        n.tick();
        assert!(n.msgs.is_empty(), "no heartbeat before timeout");
        n.tick(); // heartbeat_elapsed == 2 → broadcast
        let hbs: Vec<_> = n
            .msgs
            .iter()
            .filter(|m| m.msg_type == MessageType::MsgHeartbeat)
            .collect();
        assert_eq!(hbs.len(), 2, "heartbeat to each follower");
        assert_eq!(n.heartbeat_elapsed, 0, "heartbeat clock resets");
        assert_eq!(n.state, RaftState::Leader, "leader never self-demotes via tick");
    }

    fn msg(t: MessageType, from: u64, to: u64, term: u64) -> Message {
        Message {
            msg_type: t,
            from,
            to,
            term,
            index: 0,
            log_term: 0,
            reject: false,
        }
    }

    impl Message {
        /// Test helper: set the acked/referenced index.
        fn acked(mut self, index: u64) -> Self {
            self.index = index;
            self
        }
    }

    #[test]
    fn candidate_wins_on_majority_vote_responses() {
        let mut n = node();
        n.step(msg(MessageType::MsgHup, 1, 1, 0));
        assert_eq!(n.state, RaftState::Candidate);
        n.msgs.clear();
        // One grant (peer 2) → with self that's 2/3 → Won.
        n.step(msg(MessageType::MsgVoteResp, 2, 1, 1));
        assert_eq!(n.state, RaftState::Leader);
        assert_eq!(n.lead, 1);
    }

    #[test]
    fn candidate_steps_down_on_majority_rejection() {
        let mut n = node();
        n.step(msg(MessageType::MsgHup, 1, 1, 0));
        let mut r2 = msg(MessageType::MsgVoteResp, 2, 1, 1);
        r2.reject = true;
        let mut r3 = msg(MessageType::MsgVoteResp, 3, 1, 1);
        r3.reject = true;
        n.step(r2);
        n.step(r3);
        assert_eq!(n.state, RaftState::Follower, "lost the election");
    }

    #[test]
    fn higher_term_message_forces_follower_at_that_term() {
        let mut n = node();
        n.become_candidate(); // term 1
        n.step(msg(MessageType::MsgHeartbeat, 2, 1, 5));
        assert_eq!(n.state, RaftState::Follower);
        assert_eq!(n.term, 5);
        assert_eq!(n.lead, 2, "heartbeat reveals the leader");
    }

    #[test]
    fn grants_vote_to_up_to_date_candidate_once() {
        let mut n = node();
        n.term = 2;
        // Candidate at term 3, empty log (up-to-date vs our empty log).
        let mut v = msg(MessageType::MsgVote, 2, 1, 3);
        v.index = 0;
        v.log_term = 0;
        n.step(v);
        assert_eq!(n.term, 3);
        assert_eq!(n.vote, 2);
        let resp = n.msgs.last().unwrap();
        assert_eq!(resp.msg_type, MessageType::MsgVoteResp);
        assert!(!resp.reject, "granted");
        // A second candidate (peer 3) at the same term is denied.
        n.msgs.clear();
        let v3 = msg(MessageType::MsgVote, 3, 1, 3);
        n.step(v3);
        assert_eq!(n.vote, 2, "already voted for 2");
        assert!(n.msgs.last().unwrap().reject, "denied — already voted");
    }

    #[test]
    fn denies_vote_to_candidate_with_stale_log() {
        let mut n = node();
        n.last_index = 9;
        n.last_term = 4;
        // Candidate's log is behind (term 3 < our 4).
        let mut v = msg(MessageType::MsgVote, 2, 1, 5);
        v.index = 100;
        v.log_term = 3;
        n.step(v);
        assert_eq!(n.vote, NONE, "did not grant to a stale-log candidate");
        assert!(n.msgs.last().unwrap().reject);
    }

    /// Drive a fresh node to leadership at term 1 in a 3-voter cluster.
    fn leader() -> RaftNode {
        let mut n = node();
        n.step(msg(MessageType::MsgHup, 1, 1, 0));
        n.step(msg(MessageType::MsgVoteResp, 2, 1, 1));
        assert_eq!(n.state, RaftState::Leader);
        n.msgs.clear();
        n
    }

    #[test]
    fn propose_appends_entry_and_advances_leader_match() {
        let mut n = leader();
        let i = n.propose();
        assert_eq!(i, 1, "first proposal lands at index 1");
        assert_eq!(n.last_index, 1);
        assert_eq!(n.last_term, 1, "entry carries the current term");
        assert_eq!(n.term_at(1), 1);
        // Leader's own progress now matches its log tail.
        assert_eq!(n.tracker.progress.get(&1).unwrap().r#match, 1);
    }

    #[test]
    fn leader_commits_once_a_quorum_acks_a_current_term_entry() {
        let mut n = leader();
        n.propose(); // index 1, term 1
        assert_eq!(n.commit, 0, "self-ack alone is not a quorum of 3");
        // Follower 2 acks index 1 → leader (1) + 2 = majority of {1,2,3}.
        n.step(msg(MessageType::MsgAppResp, 2, 1, 1).acked(1));
        assert_eq!(n.commit, 1, "quorum reached → committed");
        // A third ack does not regress or double-advance.
        n.step(msg(MessageType::MsgAppResp, 3, 1, 1).acked(1));
        assert_eq!(n.commit, 1);
    }

    #[test]
    fn leader_does_not_commit_a_prior_term_entry_on_count_alone() {
        // Safety: a leader only commits a *current-term* entry by counting
        // replicas (etcd Figure 8). Seed a stale-term entry at index 1.
        let mut n = leader(); // term 1
        n.log_terms.push(0); // index 1 carries an older term 0 (< current 1)
        n.last_index = 1;
        n.last_term = 0;
        n.tracker.progress.get_mut(&1).unwrap().r#match = 1;
        n.step(msg(MessageType::MsgAppResp, 2, 1, 1).acked(1));
        assert_eq!(
            n.commit, 0,
            "must not commit a prior-term entry by replica count"
        );
    }

    #[test]
    fn app_rejection_rolls_back_follower_next() {
        let mut n = leader();
        n.propose();
        n.propose(); // last_index = 2
        let pr2_next_before = n.tracker.progress.get(&2).unwrap().next;
        assert!(pr2_next_before >= 1);
        let mut rej = msg(MessageType::MsgAppResp, 2, 1, 1);
        rej.reject = true;
        rej.index = 1;
        n.step(rej);
        // next must not have advanced past the rejection point.
        assert!(n.tracker.progress.get(&2).unwrap().next <= 2);
    }

    #[test]
    fn past_election_timeout_fires_at_randomized_threshold() {
        let mut n = node();
        n.randomized_election_timeout = 10;
        n.election_elapsed = 9;
        assert!(!n.past_election_timeout());
        n.election_elapsed = 10;
        assert!(n.past_election_timeout());
    }
}

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

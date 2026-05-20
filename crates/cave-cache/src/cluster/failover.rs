// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CLUSTER FAILOVER — replica takeover state machine.
//!
//! Ports the failover phases from upstream Redis' `clusterCommand` /
//! `clusterHandleManualFailover` paths in `src/cluster.c`. The phases:
//!
//! 1. **None** — idle.
//! 2. **WaitPause** — `CLUSTER FAILOVER` issued; the replica asks the
//!    primary to pause writes (`CLUSTER FAILOVER` without `FORCE`).
//!    With `FORCE` we skip the pause; with `TAKEOVER` we additionally
//!    skip the auth phase.
//! 3. **AwaitingAuth** — auth-request sent to the cluster's master
//!    quorum, waiting for ack votes.
//! 4. **Promoted** — auth granted (or `TAKEOVER` bypassed it); the
//!    replica self-promotes, bumps its config epoch, and starts
//!    advertising itself as primary for the inherited slots.
//! 5. **Failed** — auth timed out, primary did not pause, or epoch
//!    collision — back to `None` after a cool-down.
//!
//! Honest scope: the state machine is what an operator-driven
//! `CLUSTER FAILOVER` command exercises end-to-end. The cluster-bus
//! wire serializer for `FAILOVER_AUTH_REQUEST` / `FAILOVER_AUTH_ACK`
//! lives in `gossip.rs` — this file holds the *transitions* the
//! upstream `clusterUpdateState` cycle drives.

use std::time::{Duration, Instant};

/// Variant requested by the user issuing `CLUSTER FAILOVER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailoverMode {
    /// Default — pause primary writes, request auth from quorum.
    Graceful,
    /// `CLUSTER FAILOVER FORCE` — skip the primary-pause handshake.
    Force,
    /// `CLUSTER FAILOVER TAKEOVER` — skip auth entirely; for split-brain recovery.
    Takeover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailoverPhase {
    None,
    WaitPause,
    AwaitingAuth,
    Promoted,
    Failed,
}

#[derive(Debug, Clone)]
pub struct FailoverState {
    pub phase: FailoverPhase,
    pub mode: FailoverMode,
    pub started_at: Option<Instant>,
    pub auth_votes: u32,
    pub auth_required: u32,
    pub last_error: Option<String>,
    pub epoch_at_promotion: Option<u64>,
}

impl Default for FailoverState {
    fn default() -> Self {
        Self::new()
    }
}

impl FailoverState {
    pub fn new() -> Self {
        FailoverState {
            phase: FailoverPhase::None,
            mode: FailoverMode::Graceful,
            started_at: None,
            auth_votes: 0,
            auth_required: 0,
            last_error: None,
            epoch_at_promotion: None,
        }
    }

    /// Operator issued `CLUSTER FAILOVER` (optionally with FORCE / TAKEOVER).
    /// `quorum` is the number of master ACKs required for the auth phase.
    pub fn begin(&mut self, mode: FailoverMode, quorum: u32) {
        self.mode = mode;
        self.started_at = Some(Instant::now());
        self.auth_votes = 0;
        self.auth_required = quorum;
        self.last_error = None;
        self.epoch_at_promotion = None;
        self.phase = match mode {
            FailoverMode::Graceful => FailoverPhase::WaitPause,
            FailoverMode::Force => FailoverPhase::AwaitingAuth,
            FailoverMode::Takeover => FailoverPhase::Promoted,
        };
    }

    /// Primary acknowledged the pause (graceful path only).
    /// Transitions WaitPause → AwaitingAuth.
    pub fn primary_paused(&mut self) {
        if matches!(self.phase, FailoverPhase::WaitPause) {
            self.phase = FailoverPhase::AwaitingAuth;
        }
    }

    /// One master sent an auth ACK.  Returns true if quorum has now been reached
    /// and the replica should self-promote.
    pub fn record_auth_ack(&mut self) -> bool {
        if !matches!(self.phase, FailoverPhase::AwaitingAuth) {
            return false;
        }
        self.auth_votes = self.auth_votes.saturating_add(1);
        if self.auth_required > 0 && self.auth_votes >= self.auth_required {
            self.phase = FailoverPhase::Promoted;
            return true;
        }
        false
    }

    /// Take the inherited config epoch from the cluster-state, bumping ours
    /// to win any future epoch tiebreakers.  Called once the promotion has
    /// been decided.
    pub fn record_promotion(&mut self, new_epoch: u64) {
        self.epoch_at_promotion = Some(new_epoch);
        self.phase = FailoverPhase::Promoted;
    }

    /// Marks failover as failed (timeout, epoch collision, etc.) — keeps the
    /// reason for later observability.
    pub fn fail(&mut self, reason: impl Into<String>) {
        self.last_error = Some(reason.into());
        self.phase = FailoverPhase::Failed;
    }

    /// Periodic tick — invoked by the cluster state machine.  If we've spent
    /// longer than `timeout` in a non-terminal phase, the failover fails.
    pub fn tick(&mut self, now: Instant, timeout: Duration) {
        if matches!(self.phase, FailoverPhase::Promoted | FailoverPhase::Failed) {
            return;
        }
        if let Some(start) = self.started_at {
            if now.duration_since(start) >= timeout {
                self.fail("failover timeout");
            }
        }
    }

    /// Operator reset — releases the state machine back to idle.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.phase, FailoverPhase::Promoted | FailoverPhase::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graceful_starts_in_wait_pause_then_awaits_auth() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Graceful, 1);
        assert_eq!(fs.phase, FailoverPhase::WaitPause);
        fs.primary_paused();
        assert_eq!(fs.phase, FailoverPhase::AwaitingAuth);
    }

    #[test]
    fn force_skips_pause_phase() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Force, 1);
        assert_eq!(fs.phase, FailoverPhase::AwaitingAuth);
    }

    #[test]
    fn takeover_skips_auth_phase() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Takeover, 5);
        assert_eq!(fs.phase, FailoverPhase::Promoted);
    }

    #[test]
    fn auth_ack_promotes_at_quorum() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Force, 2);
        assert!(!fs.record_auth_ack());
        assert_eq!(fs.phase, FailoverPhase::AwaitingAuth);
        assert!(fs.record_auth_ack());
        assert_eq!(fs.phase, FailoverPhase::Promoted);
    }

    #[test]
    fn record_promotion_stores_new_epoch() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Force, 1);
        fs.record_auth_ack();
        fs.record_promotion(42);
        assert_eq!(fs.epoch_at_promotion, Some(42));
    }

    #[test]
    fn tick_fails_on_timeout() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Graceful, 1);
        let later = Instant::now() + Duration::from_secs(60);
        fs.tick(later, Duration::from_secs(1));
        assert_eq!(fs.phase, FailoverPhase::Failed);
        assert!(fs.last_error.as_deref().unwrap().contains("timeout"));
    }

    #[test]
    fn fail_records_reason() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Graceful, 1);
        fs.fail("epoch collision");
        assert_eq!(fs.phase, FailoverPhase::Failed);
        assert_eq!(fs.last_error.as_deref(), Some("epoch collision"));
    }

    #[test]
    fn reset_returns_to_idle() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Force, 1);
        fs.record_auth_ack();
        fs.reset();
        assert_eq!(fs.phase, FailoverPhase::None);
        assert_eq!(fs.auth_votes, 0);
    }

    #[test]
    fn ack_outside_auth_phase_is_ignored() {
        let mut fs = FailoverState::new();
        fs.begin(FailoverMode::Graceful, 1);
        // still in WaitPause — ACK should not transition.
        assert!(!fs.record_auth_ack());
        assert_eq!(fs.phase, FailoverPhase::WaitPause);
    }

    #[test]
    fn is_complete_true_only_in_terminal_phases() {
        let mut fs = FailoverState::new();
        assert!(!fs.is_complete());
        fs.begin(FailoverMode::Takeover, 1);
        assert!(fs.is_complete());
    }
}

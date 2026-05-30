// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Interactive-session lifecycle state machine.
//!
//! Faithful line-port of Teleport's session state machine
//! (`lib/srv/sess.go`, teleport v17.0.4). Upstream models an interactive
//! session as a small enum (`sessionState`) with a `setState` guard that
//! rejects illegal transitions, ensuring a session always advances
//! monotonically: `Pending → Running → Terminating → Terminated`.
//!
//! Upstream (Go) shape paraphrased:
//!
//! ```text
//! type sessionState int
//! const (
//!     sessionStatePending sessionState = iota
//!     sessionStateRunning
//!     sessionStateTerminating
//!     sessionStateTerminated
//! )
//!
//! func (s *session) setState(state sessionState) {
//!     // upstream guards: cannot move backwards, Terminated is a sink,
//!     // Terminated is only reachable from Terminating.
//! }
//! ```
//!
//! This is a pure in-memory algorithm: no I/O, no persistence, no clocks.
//! It closes the "connect/hangup state machine" gap previously deferred
//! from the "Session models and engine" partial (TDD 2026-05-30).

/// The lifecycle states of an interactive session, in monotonic order.
///
/// Mirrors Teleport's `sessionState` iota: lower discriminants are earlier
/// in the session's life, and a session may never move to a strictly lower
/// state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionState {
    /// Session created but not yet started (awaiting participants / approval).
    Pending,
    /// Session is live and accepting I/O.
    Running,
    /// Session is shutting down: peers detaching, recording flushing.
    Terminating,
    /// Session has fully ended. Sink state — no outbound transitions.
    Terminated,
}

impl SessionState {
    /// Numeric rank used to enforce monotonic forward progress, matching the
    /// `iota` ordering of Teleport's `sessionState`.
    fn rank(self) -> u8 {
        match self {
            SessionState::Pending => 0,
            SessionState::Running => 1,
            SessionState::Terminating => 2,
            SessionState::Terminated => 3,
        }
    }
}

/// Errors returned when a requested transition is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    /// The requested move is not permitted from the current state.
    IllegalTransition {
        from: SessionState,
        to: SessionState,
    },
}

impl std::fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LifecycleError::IllegalTransition { from, to } => {
                write!(f, "illegal session transition: {from:?} -> {to:?}")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

/// Guards and records the lifecycle of a single interactive session.
///
/// Port of the `setState` guard logic from Teleport's `session` struct: the
/// state advances monotonically, `Terminated` is a sink, and `Terminated`
/// is reachable only by first passing through `Terminating`.
#[derive(Debug, Clone)]
pub struct SessionLifecycle {
    state: SessionState,
    history: Vec<SessionState>,
}

impl Default for SessionLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionLifecycle {
    /// Create a new session in the `Pending` state (Teleport's initial state).
    pub fn new() -> Self {
        Self {
            state: SessionState::Pending,
            history: vec![SessionState::Pending],
        }
    }

    /// The session's current state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// The ordered list of every accepted state the session has occupied,
    /// starting at `Pending`. Rejected transitions are never appended.
    pub fn history(&self) -> &[SessionState] {
        &self.history
    }

    /// True once the session has reached its sink state (`Terminated`).
    pub fn is_terminal(&self) -> bool {
        self.state == SessionState::Terminated
    }

    /// Return true if `to` is a legal transition from the current state.
    ///
    /// Rules (faithful to Teleport `sess.go` `setState`):
    /// 1. A self-transition (`to == current`) is always accepted (no-op).
    /// 2. `Terminated` is a sink: no outbound transition is legal.
    /// 3. Movement must be strictly forward (rank must increase); a session
    ///    can never regress to an earlier state.
    /// 4. `Terminated` is reachable only from `Terminating` (no skipping the
    ///    drain phase from `Pending`/`Running`).
    fn can_transition(&self, to: SessionState) -> bool {
        let from = self.state;
        // (1) idempotent self-transition.
        if from == to {
            return true;
        }
        // (2) Terminated is a sink.
        if from == SessionState::Terminated {
            return false;
        }
        // (4) only Terminating may reach Terminated.
        if to == SessionState::Terminated && from != SessionState::Terminating {
            return false;
        }
        // (3) strictly forward progress only.
        to.rank() > from.rank()
    }

    /// Attempt to advance the session to `to`.
    ///
    /// On success the state is updated and (for a non-self transition)
    /// appended to `history`. On rejection the state and history are left
    /// untouched and an [`LifecycleError::IllegalTransition`] is returned.
    pub fn set_state(&mut self, to: SessionState) -> Result<(), LifecycleError> {
        if !self.can_transition(to) {
            return Err(LifecycleError::IllegalTransition {
                from: self.state,
                to,
            });
        }
        if to != self.state {
            self.state = to;
            self.history.push(to);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_happy_path() {
        let mut sm = SessionLifecycle::new();
        assert_eq!(sm.state(), SessionState::Pending);
        sm.set_state(SessionState::Running).unwrap();
        sm.set_state(SessionState::Terminating).unwrap();
        sm.set_state(SessionState::Terminated).unwrap();
        assert!(sm.is_terminal());
    }

    #[test]
    fn backward_is_rejected() {
        let mut sm = SessionLifecycle::new();
        sm.set_state(SessionState::Running).unwrap();
        assert!(sm.set_state(SessionState::Pending).is_err());
    }

    #[test]
    fn skip_to_terminated_rejected() {
        let mut sm = SessionLifecycle::new();
        assert!(sm.set_state(SessionState::Terminated).is_err());
    }
}

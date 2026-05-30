// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD (2026-05-30): session lifecycle state machine.
//!
//! Faithful line-port of Teleport's interactive session state machine
//! (`lib/srv/sess.go`: `sessionState` + `setState` legal-transition guard,
//! teleport v17.0.4). Pure in-memory algorithm — no I/O, no persistence.
//! Converts the "Session models and engine" partial's deferred
//! "connect/hangup state machine" gap into a real, tested mapped entry.

use cave_pam::session_lifecycle::{LifecycleError, SessionLifecycle, SessionState};

#[test]
fn new_session_starts_pending() {
    let sm = SessionLifecycle::new();
    assert_eq!(sm.state(), SessionState::Pending);
}

#[test]
fn pending_can_go_running() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    assert_eq!(sm.state(), SessionState::Running);
}

#[test]
fn pending_can_go_terminating() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Terminating).unwrap();
    assert_eq!(sm.state(), SessionState::Terminating);
}

#[test]
fn running_can_go_terminating() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    sm.set_state(SessionState::Terminating).unwrap();
    assert_eq!(sm.state(), SessionState::Terminating);
}

#[test]
fn terminating_can_go_terminated() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    sm.set_state(SessionState::Terminating).unwrap();
    sm.set_state(SessionState::Terminated).unwrap();
    assert_eq!(sm.state(), SessionState::Terminated);
}

#[test]
fn cannot_skip_back_from_running_to_pending() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    let err = sm.set_state(SessionState::Pending).unwrap_err();
    assert_eq!(
        err,
        LifecycleError::IllegalTransition {
            from: SessionState::Running,
            to: SessionState::Pending,
        }
    );
}

#[test]
fn cannot_resurrect_terminated_session() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    sm.set_state(SessionState::Terminating).unwrap();
    sm.set_state(SessionState::Terminated).unwrap();
    // Terminated is a sink state — every outbound transition is illegal.
    assert!(sm.set_state(SessionState::Running).is_err());
    assert!(sm.set_state(SessionState::Pending).is_err());
    assert!(sm.set_state(SessionState::Terminating).is_err());
}

#[test]
fn pending_cannot_jump_to_terminated() {
    // Teleport requires passing through Terminating before Terminated.
    let mut sm = SessionLifecycle::new();
    let err = sm.set_state(SessionState::Terminated).unwrap_err();
    assert_eq!(
        err,
        LifecycleError::IllegalTransition {
            from: SessionState::Pending,
            to: SessionState::Terminated,
        }
    );
}

#[test]
fn running_cannot_jump_to_terminated() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    assert!(sm.set_state(SessionState::Terminated).is_err());
}

#[test]
fn idempotent_self_transition_is_allowed() {
    // setState to the current state is a no-op accept (Teleport tolerates it).
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Pending).unwrap();
    assert_eq!(sm.state(), SessionState::Pending);
    sm.set_state(SessionState::Running).unwrap();
    sm.set_state(SessionState::Running).unwrap();
    assert_eq!(sm.state(), SessionState::Running);
}

#[test]
fn is_terminal_only_for_terminated() {
    let mut sm = SessionLifecycle::new();
    assert!(!sm.is_terminal());
    sm.set_state(SessionState::Running).unwrap();
    assert!(!sm.is_terminal());
    sm.set_state(SessionState::Terminating).unwrap();
    assert!(!sm.is_terminal());
    sm.set_state(SessionState::Terminated).unwrap();
    assert!(sm.is_terminal());
}

#[test]
fn history_records_every_accepted_transition() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    sm.set_state(SessionState::Terminating).unwrap();
    sm.set_state(SessionState::Terminated).unwrap();
    // history starts at Pending and appends each accepted move.
    assert_eq!(
        sm.history(),
        &[
            SessionState::Pending,
            SessionState::Running,
            SessionState::Terminating,
            SessionState::Terminated,
        ]
    );
}

#[test]
fn rejected_transition_does_not_mutate_history() {
    let mut sm = SessionLifecycle::new();
    sm.set_state(SessionState::Running).unwrap();
    let _ = sm.set_state(SessionState::Pending); // rejected
    assert_eq!(sm.history(), &[SessionState::Pending, SessionState::Running]);
}

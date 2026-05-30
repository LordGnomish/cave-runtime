// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Graceful-shutdown controller — pure-Rust port of ollama/ollama's
//! `server.Serve` signal handling (`server/routes.go`).
//!
//! Upstream catches `SIGINT` + `SIGTERM`, stops accepting new work, cancels the
//! scheduler context, drains the in-flight runner, and exits. A second signal
//! while a graceful shutdown is already underway forces an immediate exit
//! (the familiar "press Ctrl-C again to force quit" behaviour).
//!
//! This module models that lifecycle as a small, lock-free state machine so the
//! daemon loop can decide — between scheduler ticks — whether to keep ticking,
//! finish the in-flight item and stop (drain), or abort immediately.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Why a shutdown was requested. Mirrors upstream's `signal.Notify` set plus
/// the cave-local-llm stop-signal file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    /// `os.Interrupt` / Ctrl-C.
    SigInt,
    /// `syscall.SIGTERM` (launchd/systemd stop).
    SigTerm,
    /// The configured stop-signal file appeared on disk.
    StopFile,
}

impl fmt::Display for ShutdownReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShutdownReason::SigInt => f.write_str("SIGINT"),
            ShutdownReason::SigTerm => f.write_str("SIGTERM"),
            ShutdownReason::StopFile => f.write_str("stop-signal file"),
        }
    }
}

/// Lifecycle of the daemon with respect to shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownState {
    /// No shutdown requested — keep ticking.
    Running,
    /// A graceful shutdown was requested; finish the in-flight item, then exit.
    Draining(ShutdownReason),
    /// A second signal arrived during drain — abort the in-flight item now.
    Forced,
}

// State is packed into a single atomic byte so a cloned controller shared with
// the async signal-listener task observes requests without a lock.
const STATE_RUNNING: u8 = 0;
const STATE_DRAINING: u8 = 1;
const STATE_FORCED: u8 = 2;

const REASON_NONE: u8 = 0;
const REASON_SIGINT: u8 = 1;
const REASON_SIGTERM: u8 = 2;
const REASON_STOPFILE: u8 = 3;

fn encode_reason(r: ShutdownReason) -> u8 {
    match r {
        ShutdownReason::SigInt => REASON_SIGINT,
        ShutdownReason::SigTerm => REASON_SIGTERM,
        ShutdownReason::StopFile => REASON_STOPFILE,
    }
}

fn decode_reason(v: u8) -> Option<ShutdownReason> {
    match v {
        REASON_NONE => None,
        REASON_SIGINT => Some(ShutdownReason::SigInt),
        REASON_SIGTERM => Some(ShutdownReason::SigTerm),
        REASON_STOPFILE => Some(ShutdownReason::StopFile),
        _ => None,
    }
}

#[derive(Debug, Default)]
struct Inner {
    state: AtomicU8,
    reason: AtomicU8,
}

/// Cheap-to-clone handle over a shared shutdown state machine.
#[derive(Debug, Clone, Default)]
pub struct ShutdownController {
    inner: Arc<Inner>,
}

impl ShutdownController {
    /// A fresh controller in the [`ShutdownState::Running`] state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a shutdown request.
    ///
    /// The first request moves `Running -> Draining(reason)` (the original
    /// reason is preserved). Any subsequent request escalates to
    /// [`ShutdownState::Forced`] — the "press again to force quit" path.
    pub fn request(&self, reason: ShutdownReason) {
        // Claim the Running -> Draining transition exactly once.
        match self.inner.state.compare_exchange(
            STATE_RUNNING,
            STATE_DRAINING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                // We won the race to start a graceful drain; stamp the reason.
                self.inner
                    .reason
                    .store(encode_reason(reason), Ordering::SeqCst);
            }
            Err(_) => {
                // Already draining (or forced) — a second signal forces exit.
                self.inner.state.store(STATE_FORCED, Ordering::SeqCst);
            }
        }
    }

    /// The current lifecycle state.
    pub fn state(&self) -> ShutdownState {
        match self.inner.state.load(Ordering::SeqCst) {
            STATE_RUNNING => ShutdownState::Running,
            STATE_FORCED => ShutdownState::Forced,
            // STATE_DRAINING (or any unexpected value) carries a reason.
            _ => {
                let reason = decode_reason(self.inner.reason.load(Ordering::SeqCst))
                    .unwrap_or(ShutdownReason::SigTerm);
                ShutdownState::Draining(reason)
            }
        }
    }

    /// `true` once any shutdown (graceful or forced) has been requested.
    pub fn is_shutdown_requested(&self) -> bool {
        self.inner.state.load(Ordering::SeqCst) != STATE_RUNNING
    }

    /// `true` once a second signal has escalated to a forced exit — the daemon
    /// should abort the in-flight item rather than wait for it to drain.
    pub fn is_forced(&self) -> bool {
        self.inner.state.load(Ordering::SeqCst) == STATE_FORCED
    }

    /// The reason recorded by the first request, if any.
    pub fn reason(&self) -> Option<ShutdownReason> {
        decode_reason(self.inner.reason.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_controller_is_running() {
        let c = ShutdownController::new();
        assert_eq!(c.state(), ShutdownState::Running);
        assert!(!c.is_shutdown_requested());
        assert_eq!(c.reason(), None);
    }

    #[test]
    fn test_sigint_requests_graceful_drain() {
        let c = ShutdownController::new();
        c.request(ShutdownReason::SigInt);
        assert_eq!(c.state(), ShutdownState::Draining(ShutdownReason::SigInt));
        assert!(c.is_shutdown_requested());
        assert_eq!(c.reason(), Some(ShutdownReason::SigInt));
    }

    #[test]
    fn test_sigterm_requests_graceful_drain() {
        let c = ShutdownController::new();
        c.request(ShutdownReason::SigTerm);
        assert_eq!(c.state(), ShutdownState::Draining(ShutdownReason::SigTerm));
        assert_eq!(c.reason(), Some(ShutdownReason::SigTerm));
    }

    #[test]
    fn test_stop_file_requests_graceful_drain() {
        let c = ShutdownController::new();
        c.request(ShutdownReason::StopFile);
        assert_eq!(c.state(), ShutdownState::Draining(ShutdownReason::StopFile));
        assert_eq!(c.reason(), Some(ShutdownReason::StopFile));
    }

    #[test]
    fn test_controller_clone_shares_state() {
        // The daemon loop holds one handle; the signal listener task holds
        // another. A request through either must be observable by both.
        let c = ShutdownController::new();
        let listener = c.clone();
        listener.request(ShutdownReason::SigTerm);
        assert!(c.is_shutdown_requested());
        assert_eq!(c.reason(), Some(ShutdownReason::SigTerm));
    }

    #[test]
    fn test_reason_display() {
        assert_eq!(ShutdownReason::SigInt.to_string(), "SIGINT");
        assert_eq!(ShutdownReason::SigTerm.to_string(), "SIGTERM");
        assert_eq!(ShutdownReason::StopFile.to_string(), "stop-signal file");
    }

    // ── Cycle 2: double-signal force-quit escalation ────────────────────────

    #[test]
    fn test_second_signal_escalates_to_forced() {
        let c = ShutdownController::new();
        c.request(ShutdownReason::SigInt);
        assert_eq!(c.state(), ShutdownState::Draining(ShutdownReason::SigInt));
        // "Press Ctrl-C again to force quit."
        c.request(ShutdownReason::SigInt);
        assert_eq!(c.state(), ShutdownState::Forced);
        assert!(c.is_shutdown_requested());
    }

    #[test]
    fn test_first_reason_is_preserved_after_escalation() {
        let c = ShutdownController::new();
        c.request(ShutdownReason::SigTerm);
        // A different second signal still forces, but the original reason stands.
        c.request(ShutdownReason::SigInt);
        assert_eq!(c.state(), ShutdownState::Forced);
        assert_eq!(c.reason(), Some(ShutdownReason::SigTerm));
    }

    #[test]
    fn test_forced_state_is_sticky() {
        let c = ShutdownController::new();
        c.request(ShutdownReason::SigInt);
        c.request(ShutdownReason::SigTerm);
        assert_eq!(c.state(), ShutdownState::Forced);
        // Further signals never downgrade out of Forced.
        c.request(ShutdownReason::StopFile);
        c.request(ShutdownReason::SigInt);
        assert_eq!(c.state(), ShutdownState::Forced);
    }

    #[test]
    fn test_is_forced_helper() {
        let c = ShutdownController::new();
        assert!(!c.is_forced());
        c.request(ShutdownReason::SigTerm);
        assert!(!c.is_forced());
        c.request(ShutdownReason::SigTerm);
        assert!(c.is_forced());
    }

    // ── Cycle 3: loop_action drain decision table ───────────────────────────

    #[test]
    fn test_running_no_stopfile_keeps_ticking() {
        assert_eq!(loop_action(ShutdownState::Running, false), LoopAction::Tick);
    }

    #[test]
    fn test_running_with_stopfile_drains() {
        // The stop-signal file is a graceful request: finish the in-flight
        // item, then stop.
        assert_eq!(
            loop_action(ShutdownState::Running, true),
            LoopAction::DrainAndStop
        );
    }

    #[test]
    fn test_draining_drains_regardless_of_stopfile() {
        assert_eq!(
            loop_action(ShutdownState::Draining(ShutdownReason::SigTerm), false),
            LoopAction::DrainAndStop
        );
        assert_eq!(
            loop_action(ShutdownState::Draining(ShutdownReason::SigInt), true),
            LoopAction::DrainAndStop
        );
    }

    #[test]
    fn test_forced_stops_now() {
        assert_eq!(loop_action(ShutdownState::Forced, false), LoopAction::StopNow);
        assert_eq!(loop_action(ShutdownState::Forced, true), LoopAction::StopNow);
    }

    #[test]
    fn test_controller_next_action_reads_live_state() {
        // Convenience wrapper used by the daemon loop.
        let c = ShutdownController::new();
        assert_eq!(c.next_action(false), LoopAction::Tick);
        c.request(ShutdownReason::SigInt);
        assert_eq!(c.next_action(false), LoopAction::DrainAndStop);
        c.request(ShutdownReason::SigInt);
        assert_eq!(c.next_action(false), LoopAction::StopNow);
    }

    #[test]
    fn test_stopfile_seen_then_signal_still_drains() {
        // Stop-file observed (Running + true) drains; if a signal then lands the
        // controller is Draining and still drains.
        let c = ShutdownController::new();
        assert_eq!(c.next_action(true), LoopAction::DrainAndStop);
        c.request(ShutdownReason::StopFile);
        assert_eq!(c.next_action(false), LoopAction::DrainAndStop);
    }
}

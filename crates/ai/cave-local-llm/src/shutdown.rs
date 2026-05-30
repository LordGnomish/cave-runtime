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
}

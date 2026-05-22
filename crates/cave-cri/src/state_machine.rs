// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Container lifecycle state machine — strict transition validation.
//!
//! Legal transitions mirror containerd's task.go state model:
//!
//! ```text
//! Created  -> Running  (start)
//! Running  -> Paused   (pause)
//! Running  -> Stopped  (stop / kill)
//! Paused   -> Running  (unpause)
//! Paused   -> Stopped  (stop / kill)
//! Stopped  -> Running  (start restart)
//! *        -> delete   (only if not Running)
//! ```

use crate::error::{CriError, CriResult};
use crate::models::ContainerStatus;

fn validate(from: &ContainerStatus, action: &str, allowed: &[&str]) -> CriResult<()> {
    let from_str = status_name(from);
    if allowed.contains(&from_str) {
        Ok(())
    } else {
        Err(CriError::InvalidState(format!(
            "cannot {} container in {} state (allowed from: {})",
            action,
            from_str,
            allowed.join(", ")
        )))
    }
}

fn status_name(s: &ContainerStatus) -> &'static str {
    match s {
        ContainerStatus::Created => "Created",
        ContainerStatus::Running => "Running",
        ContainerStatus::Paused => "Paused",
        ContainerStatus::Stopped => "Stopped",
        ContainerStatus::Failed(_) => "Failed",
    }
}

pub fn check_start(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "start", &["Created", "Stopped"])
}

pub fn check_stop(status: &ContainerStatus) -> CriResult<()> {
    match status {
        ContainerStatus::Running | ContainerStatus::Paused => Ok(()),
        ContainerStatus::Stopped => Ok(()), // idempotent
        _ => validate(status, "stop", &["Running", "Paused", "Stopped"]),
    }
}

pub fn check_kill(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "kill", &["Running", "Paused"])
}

pub fn check_pause(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "pause", &["Running"])
}

pub fn check_unpause(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "unpause", &["Paused"])
}

pub fn check_exec(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "exec in", &["Running"])
}

pub fn check_delete(status: &ContainerStatus) -> CriResult<()> {
    match status {
        ContainerStatus::Running => Err(CriError::InvalidState(
            "cannot delete running container — stop it first".into(),
        )),
        _ => Ok(()),
    }
}

pub fn check_checkpoint(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "checkpoint", &["Running"])
}

pub fn check_restore(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "restore", &["Stopped", "Created"])
}

pub fn check_logs(status: &ContainerStatus) -> CriResult<()> {
    match status {
        ContainerStatus::Running | ContainerStatus::Stopped | ContainerStatus::Paused => Ok(()),
        _ => validate(status, "get logs from", &["Running", "Stopped", "Paused"]),
    }
}

pub fn check_processes(status: &ContainerStatus) -> CriResult<()> {
    validate(status, "list processes in", &["Running"])
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── start ──────────────────────────────────────────────────────────────────

    #[test]
    fn start_from_created_ok() {
        assert!(check_start(&ContainerStatus::Created).is_ok());
    }

    #[test]
    fn start_from_stopped_ok() {
        assert!(check_start(&ContainerStatus::Stopped).is_ok());
    }

    #[test]
    fn start_from_running_err() {
        let e = check_start(&ContainerStatus::Running).unwrap_err();
        assert!(e.to_string().contains("start"));
        assert!(e.to_string().contains("Running"));
    }

    #[test]
    fn start_from_paused_err() {
        assert!(check_start(&ContainerStatus::Paused).is_err());
    }

    #[test]
    fn start_from_failed_err() {
        assert!(check_start(&ContainerStatus::Failed("oom".into())).is_err());
    }

    // ── stop ───────────────────────────────────────────────────────────────────

    #[test]
    fn stop_from_running_ok() {
        assert!(check_stop(&ContainerStatus::Running).is_ok());
    }

    #[test]
    fn stop_from_paused_ok() {
        assert!(check_stop(&ContainerStatus::Paused).is_ok());
    }

    #[test]
    fn stop_from_stopped_ok_idempotent() {
        assert!(check_stop(&ContainerStatus::Stopped).is_ok());
    }

    #[test]
    fn stop_from_created_err() {
        assert!(check_stop(&ContainerStatus::Created).is_err());
    }

    // ── kill ───────────────────────────────────────────────────────────────────

    #[test]
    fn kill_from_running_ok() {
        assert!(check_kill(&ContainerStatus::Running).is_ok());
    }

    #[test]
    fn kill_from_paused_ok() {
        assert!(check_kill(&ContainerStatus::Paused).is_ok());
    }

    #[test]
    fn kill_from_stopped_err() {
        assert!(check_kill(&ContainerStatus::Stopped).is_err());
    }

    #[test]
    fn kill_from_created_err() {
        assert!(check_kill(&ContainerStatus::Created).is_err());
    }

    // ── pause ──────────────────────────────────────────────────────────────────

    #[test]
    fn pause_from_running_ok() {
        assert!(check_pause(&ContainerStatus::Running).is_ok());
    }

    #[test]
    fn pause_from_paused_err() {
        let e = check_pause(&ContainerStatus::Paused).unwrap_err();
        assert!(e.to_string().contains("pause"));
    }

    #[test]
    fn pause_from_stopped_err() {
        assert!(check_pause(&ContainerStatus::Stopped).is_err());
    }

    #[test]
    fn pause_from_created_err() {
        assert!(check_pause(&ContainerStatus::Created).is_err());
    }

    // ── unpause ────────────────────────────────────────────────────────────────

    #[test]
    fn unpause_from_paused_ok() {
        assert!(check_unpause(&ContainerStatus::Paused).is_ok());
    }

    #[test]
    fn unpause_from_running_err() {
        let e = check_unpause(&ContainerStatus::Running).unwrap_err();
        assert!(e.to_string().contains("unpause"));
    }

    #[test]
    fn unpause_from_stopped_err() {
        assert!(check_unpause(&ContainerStatus::Stopped).is_err());
    }

    #[test]
    fn unpause_from_created_err() {
        assert!(check_unpause(&ContainerStatus::Created).is_err());
    }

    // ── exec ───────────────────────────────────────────────────────────────────

    #[test]
    fn exec_from_running_ok() {
        assert!(check_exec(&ContainerStatus::Running).is_ok());
    }

    #[test]
    fn exec_from_stopped_err() {
        assert!(check_exec(&ContainerStatus::Stopped).is_err());
    }

    #[test]
    fn exec_from_paused_err() {
        assert!(check_exec(&ContainerStatus::Paused).is_err());
    }

    #[test]
    fn exec_from_created_err() {
        assert!(check_exec(&ContainerStatus::Created).is_err());
    }

    // ── delete ─────────────────────────────────────────────────────────────────

    #[test]
    fn delete_running_err() {
        let e = check_delete(&ContainerStatus::Running).unwrap_err();
        assert!(e.to_string().contains("delete"));
    }

    #[test]
    fn delete_stopped_ok() {
        assert!(check_delete(&ContainerStatus::Stopped).is_ok());
    }

    #[test]
    fn delete_created_ok() {
        assert!(check_delete(&ContainerStatus::Created).is_ok());
    }

    #[test]
    fn delete_paused_ok() {
        assert!(check_delete(&ContainerStatus::Paused).is_ok());
    }

    #[test]
    fn delete_failed_ok() {
        assert!(check_delete(&ContainerStatus::Failed("x".into())).is_ok());
    }

    // ── checkpoint / restore ───────────────────────────────────────────────────

    #[test]
    fn checkpoint_running_ok() {
        assert!(check_checkpoint(&ContainerStatus::Running).is_ok());
    }

    #[test]
    fn checkpoint_stopped_err() {
        assert!(check_checkpoint(&ContainerStatus::Stopped).is_err());
    }

    #[test]
    fn restore_stopped_ok() {
        assert!(check_restore(&ContainerStatus::Stopped).is_ok());
    }

    #[test]
    fn restore_created_ok() {
        assert!(check_restore(&ContainerStatus::Created).is_ok());
    }

    #[test]
    fn restore_running_err() {
        assert!(check_restore(&ContainerStatus::Running).is_err());
    }

    // ── processes ──────────────────────────────────────────────────────────────

    #[test]
    fn processes_running_ok() {
        assert!(check_processes(&ContainerStatus::Running).is_ok());
    }

    #[test]
    fn processes_stopped_err() {
        assert!(check_processes(&ContainerStatus::Stopped).is_err());
    }

    // ── error message quality ──────────────────────────────────────────────────

    #[test]
    fn error_message_contains_state_and_action() {
        let e = check_start(&ContainerStatus::Running).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("start"), "missing action in: {}", msg);
        assert!(msg.contains("Running"), "missing state in: {}", msg);
    }

    #[test]
    fn error_message_contains_allowed_states() {
        let e = check_pause(&ContainerStatus::Stopped).unwrap_err();
        let msg = e.to_string();
        assert!(
            msg.contains("Running"),
            "allowed states missing from: {}",
            msg
        );
    }
}

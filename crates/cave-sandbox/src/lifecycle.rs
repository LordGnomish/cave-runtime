// SPDX-License-Identifier: AGPL-3.0-or-later
//! Unified sandbox lifecycle — state machine across all three runtimes.
//!
//! `Created → Running → Paused ⇄ Running → Stopped → Removed`.

use crate::models::{Runtime, SandboxState};
use serde::{Deserialize, Serialize};

/// Extended state shared across gVisor/Kata/Firecracker.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum LifecycleState {
    Created,
    Running,
    Paused,
    Stopped,
    Removed,
}

impl LifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleState::Created => "created",
            LifecycleState::Running => "running",
            LifecycleState::Paused => "paused",
            LifecycleState::Stopped => "stopped",
            LifecycleState::Removed => "removed",
        }
    }

    /// Legal transitions — runtime-agnostic.
    pub fn can_transition(self, to: LifecycleState) -> bool {
        use LifecycleState::*;
        matches!(
            (self, to),
            (Created, Running)
                | (Created, Stopped)
                | (Running, Paused)
                | (Paused, Running)
                | (Running, Stopped)
                | (Paused, Stopped)
                | (Stopped, Removed)
        )
    }

    /// Map to the canonical Sandbox API state.
    pub fn to_sandbox_state(self) -> SandboxState {
        match self {
            LifecycleState::Created => SandboxState::Created,
            LifecycleState::Running => SandboxState::Running,
            LifecycleState::Paused => SandboxState::Paused,
            LifecycleState::Stopped | LifecycleState::Removed => SandboxState::Stopped,
        }
    }
}

/// Audit-trail entry for an in-store sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifecycleEvent {
    pub at: chrono::DateTime<chrono::Utc>,
    pub from: LifecycleState,
    pub to: LifecycleState,
    pub runtime: Runtime,
    pub reason: Option<String>,
}

/// State machine with a transition log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifecycleMachine {
    pub state: LifecycleState,
    pub runtime: Runtime,
    pub history: Vec<LifecycleEvent>,
}

impl LifecycleMachine {
    pub fn new(runtime: Runtime) -> Self {
        LifecycleMachine { state: LifecycleState::Created, runtime, history: Vec::new() }
    }

    /// Drive a transition; on illegal moves return Err and leave state alone.
    pub fn transition(&mut self, to: LifecycleState, reason: Option<String>) -> Result<(), String> {
        if !self.state.can_transition(to) {
            return Err(format!("illegal transition {:?} → {:?}", self.state, to));
        }
        let event = LifecycleEvent {
            at: chrono::Utc::now(),
            from: self.state,
            to,
            runtime: self.runtime.clone(),
            reason,
        };
        self.state = to;
        self.history.push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_strings_lowercase() {
        assert_eq!(LifecycleState::Running.as_str(), "running");
        assert_eq!(LifecycleState::Removed.as_str(), "removed");
    }

    #[test]
    fn happy_path() {
        let mut m = LifecycleMachine::new(Runtime::Gvisor);
        m.transition(LifecycleState::Running, None).unwrap();
        m.transition(LifecycleState::Paused, None).unwrap();
        m.transition(LifecycleState::Running, None).unwrap();
        m.transition(LifecycleState::Stopped, Some("oci-exit".into())).unwrap();
        m.transition(LifecycleState::Removed, None).unwrap();
        assert_eq!(m.state, LifecycleState::Removed);
        assert_eq!(m.history.len(), 5);
    }

    #[test]
    fn illegal_moves_rejected() {
        let mut m = LifecycleMachine::new(Runtime::Kata);
        let err = m.transition(LifecycleState::Paused, None);
        assert!(err.is_err());
        assert_eq!(m.state, LifecycleState::Created);
        assert!(m.history.is_empty());
    }

    #[test]
    fn removed_is_terminal() {
        let mut m = LifecycleMachine::new(Runtime::Firecracker);
        m.transition(LifecycleState::Running, None).unwrap();
        m.transition(LifecycleState::Stopped, None).unwrap();
        m.transition(LifecycleState::Removed, None).unwrap();
        assert!(m.transition(LifecycleState::Running, None).is_err());
    }

    #[test]
    fn to_sandbox_state_maps_stopped() {
        assert_eq!(LifecycleState::Stopped.to_sandbox_state(), SandboxState::Stopped);
        assert_eq!(LifecycleState::Removed.to_sandbox_state(), SandboxState::Stopped);
        assert_eq!(LifecycleState::Running.to_sandbox_state(), SandboxState::Running);
    }

    #[test]
    fn history_records_reason() {
        let mut m = LifecycleMachine::new(Runtime::Gvisor);
        m.transition(LifecycleState::Running, Some("user-start".into())).unwrap();
        assert_eq!(m.history[0].reason.as_deref(), Some("user-start"));
    }

    #[test]
    fn created_to_stopped_skip_running() {
        let mut m = LifecycleMachine::new(Runtime::Gvisor);
        assert!(m.transition(LifecycleState::Stopped, None).is_ok());
    }
}

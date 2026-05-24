// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: command surface adapted from google/gvisor (Apache-2.0).
//
// gVisor source: `runsc/cmd/*.go` (cmd/create.go, cmd/start.go, cmd/kill.go,
// cmd/delete.go, cmd/state.go, cmd/exec.go, cmd/list.go).
//! gVisor `runsc` command surface — control-plane model.
//!
//! `runsc` is gVisor's OCI runtime binary. This module ports the *command
//! surface* and lifecycle state machine. Actual ptrace/KVM/systrap kernel
//! interaction is OUT OF SCOPE (see scope_cuts in parity.manifest.toml).

use crate::oci_runtime_spec::Spec;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// State machine: `Creating → Created → Running → Stopped`.
/// Mirrors `pkg/sentry/control/state.go` State enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RunscState {
    Creating,
    Created,
    Running,
    Paused,
    Stopped,
}

impl RunscState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunscState::Creating => "creating",
            RunscState::Created => "created",
            RunscState::Running => "running",
            RunscState::Paused => "paused",
            RunscState::Stopped => "stopped",
        }
    }

    /// Legal transitions per `runsc/container/container.go`.
    pub fn can_transition(self, to: RunscState) -> bool {
        use RunscState::*;
        matches!(
            (self, to),
            (Creating, Created)
                | (Created, Running)
                | (Created, Stopped)
                | (Running, Paused)
                | (Paused, Running)
                | (Running, Stopped)
                | (Paused, Stopped)
        )
    }
}

/// A managed runsc container — mirrors `runsc/container/container.go`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunscContainer {
    pub id: String,
    pub bundle: String,
    pub spec: Spec,
    pub state: RunscState,
    pub created_at: DateTime<Utc>,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
}

impl RunscContainer {
    pub fn new(id: impl Into<String>, bundle: impl Into<String>, spec: Spec) -> Self {
        RunscContainer {
            id: id.into(),
            bundle: bundle.into(),
            spec,
            state: RunscState::Creating,
            created_at: Utc::now(),
            pid: None,
            exit_code: None,
        }
    }
}

/// `runsc state <id>` output — JSON written by runsc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunscStateReport {
    pub oci_version: String,
    pub id: String,
    pub status: String,
    pub pid: u32,
    pub bundle: String,
    pub annotations: std::collections::BTreeMap<String, String>,
}

/// `runsc exec` parameters — `runsc/cmd/exec.go`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ExecRequest {
    pub container_id: String,
    pub argv: Vec<String>,
    pub env: Vec<String>,
    pub cwd: Option<String>,
    pub uid: u32,
    pub gid: u32,
    pub tty: bool,
}

/// `runsc list` output — `runsc/cmd/list.go`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListEntry {
    pub id: String,
    pub pid: u32,
    pub status: String,
    pub bundle: String,
    pub created: DateTime<Utc>,
    pub owner: String,
}

/// Runsc command surface — operations a control plane invokes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RunscCommand {
    Create { id: String, bundle: String },
    Start { id: String },
    Kill { id: String, signal: i32 },
    Delete { id: String, force: bool },
    State { id: String },
    Exec(ExecRequest),
    List,
    Pause { id: String },
    Resume { id: String },
    Events { id: String, stats: bool },
    Wait { id: String },
}

impl RunscCommand {
    /// CLI argv for the `runsc` binary — used to render `cavectl sandbox
    /// debug runsc-args …` and for parity tests.
    pub fn to_argv(&self) -> Vec<String> {
        match self {
            RunscCommand::Create { id, bundle } => {
                vec!["create".into(), "--bundle".into(), bundle.clone(), id.clone()]
            }
            RunscCommand::Start { id } => vec!["start".into(), id.clone()],
            RunscCommand::Kill { id, signal } => {
                vec!["kill".into(), id.clone(), signal.to_string()]
            }
            RunscCommand::Delete { id, force } => {
                let mut v = vec!["delete".into()];
                if *force {
                    v.push("--force".into());
                }
                v.push(id.clone());
                v
            }
            RunscCommand::State { id } => vec!["state".into(), id.clone()],
            RunscCommand::Exec(req) => {
                let mut v = vec!["exec".into(), req.container_id.clone()];
                v.extend(req.argv.iter().cloned());
                v
            }
            RunscCommand::List => vec!["list".into()],
            RunscCommand::Pause { id } => vec!["pause".into(), id.clone()],
            RunscCommand::Resume { id } => vec!["resume".into(), id.clone()],
            RunscCommand::Events { id, stats } => {
                let mut v = vec!["events".into()];
                if *stats {
                    v.push("--stats".into());
                }
                v.push(id.clone());
                v
            }
            RunscCommand::Wait { id } => vec!["wait".into(), id.clone()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_transitions_legal_chain() {
        use RunscState::*;
        assert!(Creating.can_transition(Created));
        assert!(Created.can_transition(Running));
        assert!(Running.can_transition(Paused));
        assert!(Paused.can_transition(Running));
        assert!(Running.can_transition(Stopped));
    }

    #[test]
    fn state_transitions_illegal_rejected() {
        use RunscState::*;
        assert!(!Stopped.can_transition(Running));
        assert!(!Creating.can_transition(Running));
        assert!(!Paused.can_transition(Created));
    }

    #[test]
    fn state_str_lowercase() {
        assert_eq!(RunscState::Running.as_str(), "running");
        assert_eq!(RunscState::Stopped.as_str(), "stopped");
    }

    #[test]
    fn create_argv() {
        let c = RunscCommand::Create { id: "c1".into(), bundle: "/b".into() };
        assert_eq!(
            c.to_argv(),
            vec!["create".to_string(), "--bundle".into(), "/b".into(), "c1".into()]
        );
    }

    #[test]
    fn kill_argv_includes_signal() {
        let c = RunscCommand::Kill { id: "c1".into(), signal: 9 };
        assert_eq!(c.to_argv(), vec!["kill", "c1", "9"]);
    }

    #[test]
    fn delete_force_flag() {
        let c = RunscCommand::Delete { id: "c1".into(), force: true };
        assert_eq!(c.to_argv(), vec!["delete", "--force", "c1"]);
    }

    #[test]
    fn exec_argv_appends_command() {
        let c = RunscCommand::Exec(ExecRequest {
            container_id: "c1".into(),
            argv: vec!["ls".into(), "-la".into()],
            ..ExecRequest::default()
        });
        assert_eq!(c.to_argv(), vec!["exec", "c1", "ls", "-la"]);
    }

    #[test]
    fn list_argv_no_args() {
        assert_eq!(RunscCommand::List.to_argv(), vec!["list"]);
    }

    #[test]
    fn container_starts_in_creating() {
        let c = RunscContainer::new("x", "/b", Spec::minimal_shell("/r"));
        assert_eq!(c.state, RunscState::Creating);
        assert!(c.pid.is_none());
    }
}

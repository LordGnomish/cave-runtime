// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: command surface adapted from kata-containers/kata-containers
// src/runtime/cmd/kata-runtime/* + src/runtime/virtcontainers/* (Apache-2.0).
//! kata-runtime — OCI runtime that boots a lightweight VM per pod.
//!
//! This module ports the *command surface* and sandbox/container state machine.
//! Actual hypervisor spawn, vsock RPC to the kata-agent, and CNI calls are
//! OUT OF SCOPE (see scope_cuts).

use crate::oci_runtime_spec::Spec;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Sandbox state — `virtcontainers/sandbox.go::SandboxState`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum KataSandboxState {
    Ready,
    Running,
    Paused,
    Stopped,
}

impl KataSandboxState {
    pub fn as_str(self) -> &'static str {
        match self {
            KataSandboxState::Ready => "ready",
            KataSandboxState::Running => "running",
            KataSandboxState::Paused => "paused",
            KataSandboxState::Stopped => "stopped",
        }
    }
}

/// Container state inside a sandbox.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum KataContainerState {
    Created,
    Running,
    Paused,
    Stopped,
}

/// Agent configuration — `src/agent/src/config.rs` and runtime side.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    /// vsock CID for the host-agent channel.
    pub vsock_cid: u32,
    /// vsock port (default 1024).
    pub vsock_port: u32,
    /// gRPC timeout seconds.
    pub grpc_timeout_secs: u64,
    /// Tracing on (debug builds).
    pub tracing: bool,
    /// Kernel cmdline tail forwarded to the agent.
    pub kernel_modules: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            vsock_cid: 3,
            vsock_port: 1024,
            grpc_timeout_secs: 30,
            tracing: false,
            kernel_modules: Vec::new(),
        }
    }
}

/// kata sandbox object — `virtcontainers/sandbox.go::Sandbox`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KataSandbox {
    pub id: String,
    pub spec: Spec,
    pub state: KataSandboxState,
    pub agent: AgentConfig,
    pub created_at: DateTime<Utc>,
    pub containers: Vec<KataContainer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KataContainer {
    pub id: String,
    pub state: KataContainerState,
    pub bundle: String,
    pub pid: Option<u32>,
}

impl KataSandbox {
    pub fn new(id: impl Into<String>, spec: Spec) -> Self {
        KataSandbox {
            id: id.into(),
            spec,
            state: KataSandboxState::Ready,
            agent: AgentConfig::default(),
            created_at: Utc::now(),
            containers: Vec::new(),
        }
    }

    pub fn add_container(&mut self, c: KataContainer) {
        self.containers.push(c);
    }
}

/// kata-runtime CLI command surface — `cmd/kata-runtime/*.go`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum KataCommand {
    Create { id: String, bundle: String },
    Start { id: String },
    Exec { id: String, argv: Vec<String> },
    Kill { id: String, signal: i32 },
    Delete { id: String, force: bool },
    State { id: String },
    List,
    Pause { id: String },
    Resume { id: String },
    /// `kata-runtime check` — host-readiness checks.
    Check,
    /// `kata-runtime env` — runtime env JSON dump.
    Env,
}

impl KataCommand {
    pub fn to_argv(&self) -> Vec<String> {
        match self {
            KataCommand::Create { id, bundle } => {
                vec!["create".into(), "--bundle".into(), bundle.clone(), id.clone()]
            }
            KataCommand::Start { id } => vec!["start".into(), id.clone()],
            KataCommand::Exec { id, argv } => {
                let mut v = vec!["exec".into(), id.clone()];
                v.extend(argv.iter().cloned());
                v
            }
            KataCommand::Kill { id, signal } => vec!["kill".into(), id.clone(), signal.to_string()],
            KataCommand::Delete { id, force } => {
                let mut v = vec!["delete".into()];
                if *force {
                    v.push("--force".into());
                }
                v.push(id.clone());
                v
            }
            KataCommand::State { id } => vec!["state".into(), id.clone()],
            KataCommand::List => vec!["list".into()],
            KataCommand::Pause { id } => vec!["pause".into(), id.clone()],
            KataCommand::Resume { id } => vec!["resume".into(), id.clone()],
            KataCommand::Check => vec!["check".into()],
            KataCommand::Env => vec!["env".into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_starts_ready() {
        let s = KataSandbox::new("s1", Spec::minimal_shell("/r"));
        assert_eq!(s.state, KataSandboxState::Ready);
        assert!(s.containers.is_empty());
    }

    #[test]
    fn sandbox_state_str() {
        assert_eq!(KataSandboxState::Running.as_str(), "running");
        assert_eq!(KataSandboxState::Paused.as_str(), "paused");
    }

    #[test]
    fn add_container() {
        let mut s = KataSandbox::new("s", Spec::default());
        s.add_container(KataContainer {
            id: "c1".into(),
            state: KataContainerState::Created,
            bundle: "/b".into(),
            pid: None,
        });
        assert_eq!(s.containers.len(), 1);
    }

    #[test]
    fn agent_default_vsock() {
        let a = AgentConfig::default();
        assert_eq!(a.vsock_cid, 3);
        assert_eq!(a.vsock_port, 1024);
    }

    #[test]
    fn create_argv() {
        let c = KataCommand::Create { id: "c".into(), bundle: "/b".into() };
        assert_eq!(c.to_argv(), vec!["create", "--bundle", "/b", "c"]);
    }

    #[test]
    fn check_env_argv() {
        assert_eq!(KataCommand::Check.to_argv(), vec!["check"]);
        assert_eq!(KataCommand::Env.to_argv(), vec!["env"]);
    }

    #[test]
    fn delete_force() {
        let c = KataCommand::Delete { id: "c".into(), force: true };
        assert_eq!(c.to_argv(), vec!["delete", "--force", "c"]);
    }

    #[test]
    fn exec_argv() {
        let c = KataCommand::Exec { id: "c".into(), argv: vec!["sh".into()] };
        assert_eq!(c.to_argv(), vec!["exec", "c", "sh"]);
    }
}

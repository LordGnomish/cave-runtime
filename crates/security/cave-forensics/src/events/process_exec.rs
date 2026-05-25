// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Process exec/exit events.
//!
//! Upstream: `pkg/grpc/exec/exec.go`, `api/v1/tetragon/tetragon.proto::ProcessExec`,
//! `pkg/exec/exec.go`.

use crate::process::Process;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// `ProcessExec` event — emitted when the kernel sched_process_exec hook
/// fires. Contains the new process + a snapshot of ancestor exec_ids
/// for offline reconstruction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessExecEvent {
    pub process: Process,
    pub ancestors: Vec<String>,
    pub observed_at: DateTime<Utc>,
}

/// `ProcessExit` event — emitted on sched_process_exit. Carries the exit
/// signal + status code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessExitEvent {
    pub process: Process,
    pub status: i32,
    pub signal: Option<i32>,
    pub observed_at: DateTime<Utc>,
}

impl ProcessExecEvent {
    /// True if the new process was exec'd from a shell binary
    /// (`/bin/bash`, `/bin/sh`, `/usr/bin/zsh`, ...). Tetragon uses this
    /// signal heavily for "container escape via shell" detection.
    pub fn is_shell(&self) -> bool {
        matches!(
            self.process.binary.as_str(),
            "/bin/sh"
                | "/bin/bash"
                | "/bin/dash"
                | "/bin/ash"
                | "/usr/bin/sh"
                | "/usr/bin/bash"
                | "/usr/bin/zsh"
                | "/usr/bin/dash"
                | "/usr/bin/ash"
        )
    }

    /// Convenience — depth of the process in the ancestry chain.
    pub fn depth(&self) -> usize {
        self.ancestors.len()
    }
}

impl ProcessExitEvent {
    /// True if exit was due to a signal (SIGKILL, SIGSEGV, etc.).
    pub fn killed_by_signal(&self) -> bool {
        self.signal.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Credentials, Namespaces};
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    fn proc(bin: &str) -> Process {
        Process {
            exec_id: "x".into(),
            pid: 1,
            pid_in_ns: 1,
            binary: bin.into(),
            arguments: String::new(),
            cwd: "/".into(),
            credentials: Credentials::default(),
            namespaces: Namespaces::default(),
            parent_exec_id: None,
            container_id: None,
            pod_name: None,
            pod_namespace: None,
            start_time: ts(),
            end_time: None,
        }
    }

    #[test]
    fn test_is_shell_positive() {
        let ev = ProcessExecEvent {
            process: proc("/bin/bash"),
            ancestors: vec![],
            observed_at: ts(),
        };
        assert!(ev.is_shell());
    }

    #[test]
    fn test_is_shell_negative() {
        let ev = ProcessExecEvent {
            process: proc("/usr/bin/curl"),
            ancestors: vec![],
            observed_at: ts(),
        };
        assert!(!ev.is_shell());
    }

    #[test]
    fn test_depth_counts_ancestors() {
        let ev = ProcessExecEvent {
            process: proc("/bin/sh"),
            ancestors: vec!["a".into(), "b".into(), "c".into()],
            observed_at: ts(),
        };
        assert_eq!(ev.depth(), 3);
    }

    #[test]
    fn test_killed_by_signal_true_when_signal_set() {
        let ev = ProcessExitEvent {
            process: proc("/bin/true"),
            status: 0,
            signal: Some(9),
            observed_at: ts(),
        };
        assert!(ev.killed_by_signal());
    }

    #[test]
    fn test_killed_by_signal_false_when_clean_exit() {
        let ev = ProcessExitEvent {
            process: proc("/bin/true"),
            status: 0,
            signal: None,
            observed_at: ts(),
        };
        assert!(!ev.killed_by_signal());
    }

    #[test]
    fn test_process_exec_serde_roundtrip() {
        let ev = ProcessExecEvent {
            process: proc("/bin/sh"),
            ancestors: vec!["p".into()],
            observed_at: ts(),
        };
        let j = serde_json::to_string(&ev).unwrap();
        let back: ProcessExecEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn test_process_exit_serde_roundtrip() {
        let ev = ProcessExitEvent {
            process: proc("/bin/sh"),
            status: 137,
            signal: Some(9),
            observed_at: ts(),
        };
        let j = serde_json::to_string(&ev).unwrap();
        let back: ProcessExitEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, ev);
    }
}

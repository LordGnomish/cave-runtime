// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: containerd shim v2 API adapted from kata-containers
// src/runtime/cmd/containerd-shim-kata-v2/* (Apache-2.0).
//! containerd-shim-kata-v2 — Task Service v2 API.
//!
//! Data model only — the actual ttrpc transport is OUT OF SCOPE.

use serde::{Deserialize, Serialize};

/// `task.TaskService` method enum — `runtime/v2/task/shim.proto`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ShimMethod {
    State,
    Create,
    Start,
    Delete,
    Pids,
    Pause,
    Resume,
    Checkpoint,
    Kill,
    Exec,
    ResizePty,
    CloseIO,
    Update,
    Wait,
    Stats,
    Connect,
    Shutdown,
}

impl ShimMethod {
    pub fn method_name(&self) -> &'static str {
        match self {
            ShimMethod::State => "State",
            ShimMethod::Create => "Create",
            ShimMethod::Start => "Start",
            ShimMethod::Delete => "Delete",
            ShimMethod::Pids => "Pids",
            ShimMethod::Pause => "Pause",
            ShimMethod::Resume => "Resume",
            ShimMethod::Checkpoint => "Checkpoint",
            ShimMethod::Kill => "Kill",
            ShimMethod::Exec => "Exec",
            ShimMethod::ResizePty => "ResizePty",
            ShimMethod::CloseIO => "CloseIO",
            ShimMethod::Update => "Update",
            ShimMethod::Wait => "Wait",
            ShimMethod::Stats => "Stats",
            ShimMethod::Connect => "Connect",
            ShimMethod::Shutdown => "Shutdown",
        }
    }
}

/// `task.CreateTaskRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CreateTaskRequest {
    pub id: String,
    pub bundle: String,
    pub rootfs: Vec<RootfsMount>,
    pub terminal: bool,
    pub stdin: String,
    pub stdout: String,
    pub stderr: String,
    pub checkpoint: String,
    pub parent_checkpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RootfsMount {
    #[serde(rename = "type")]
    pub fs_type: String,
    pub source: String,
    pub options: Vec<String>,
}

/// `task.StateResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StateResponse {
    pub id: String,
    pub bundle: String,
    pub pid: u32,
    pub status: String,
    pub stdin: String,
    pub stdout: String,
    pub stderr: String,
    pub terminal: bool,
    pub exit_status: u32,
    pub exited_at: Option<chrono::DateTime<chrono::Utc>>,
    pub exec_id: String,
}

/// Shim address — `/containerd/io.containerd.runtime.v2.task/<ns>/<id>/shim.sock`.
pub fn shim_socket(ns: &str, id: &str) -> String {
    format!("/run/containerd/io.containerd.runtime.v2.task/{ns}/{id}/shim.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_method_count_seventeen() {
        let methods = [
            ShimMethod::State, ShimMethod::Create, ShimMethod::Start, ShimMethod::Delete,
            ShimMethod::Pids, ShimMethod::Pause, ShimMethod::Resume, ShimMethod::Checkpoint,
            ShimMethod::Kill, ShimMethod::Exec, ShimMethod::ResizePty, ShimMethod::CloseIO,
            ShimMethod::Update, ShimMethod::Wait, ShimMethod::Stats, ShimMethod::Connect,
            ShimMethod::Shutdown,
        ];
        assert_eq!(methods.len(), 17);
        for m in methods { assert!(!m.method_name().is_empty()); }
    }

    #[test]
    fn create_task_default() {
        let r = CreateTaskRequest::default();
        assert!(r.id.is_empty());
    }

    #[test]
    fn state_response_roundtrip() {
        let s = StateResponse {
            id: "task-1".into(),
            bundle: "/b".into(),
            pid: 1234,
            status: "running".into(),
            exit_status: 0,
            ..StateResponse::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: StateResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(back.pid, 1234);
        assert_eq!(back.status, "running");
    }

    #[test]
    fn shim_socket_layout() {
        let s = shim_socket("k8s.io", "abc");
        assert!(s.contains("io.containerd.runtime.v2.task"));
        assert!(s.ends_with("/abc/shim.sock"));
    }

    #[test]
    fn rootfs_mount_serializes_type_alias() {
        let r = RootfsMount {
            fs_type: "overlay".into(),
            source: "overlay".into(),
            options: vec!["lowerdir=/a".into()],
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"type\":\"overlay\""));
    }

    #[test]
    fn method_names_pascalcase() {
        assert_eq!(ShimMethod::CloseIO.method_name(), "CloseIO");
        assert_eq!(ShimMethod::ResizePty.method_name(), "ResizePty");
    }
}

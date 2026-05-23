// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kernel-event taxonomy. One module per syscall family.
//!
//! Upstream: `pkg/grpc/exec/exec.go`, `pkg/observer/observer.go`,
//! `pkg/grpc/tracing/{kprobe,tracepoint,uprobe}.go`, `api/v1/tetragon/events.proto`.

pub mod bpf;
pub mod capability;
pub mod file;
pub mod kprobe;
pub mod network;
pub mod process_exec;

use crate::process::Process;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Top-level kernel event envelope — every event Tetragon emits gets
/// flattened into one of these variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelEvent {
    ProcessExec(process_exec::ProcessExecEvent),
    ProcessExit(process_exec::ProcessExitEvent),
    FileOp(file::FileEvent),
    Network(network::NetworkEvent),
    Capability(capability::CapabilityCheckEvent),
    BpfLoad(bpf::BpfLoadEvent),
    Kprobe(kprobe::KprobeEvent),
    Uprobe(kprobe::UprobeEvent),
}

impl KernelEvent {
    /// Wall-clock time the kernel observed the event.
    pub fn observed_at(&self) -> DateTime<Utc> {
        match self {
            KernelEvent::ProcessExec(e) => e.observed_at,
            KernelEvent::ProcessExit(e) => e.observed_at,
            KernelEvent::FileOp(e) => e.observed_at,
            KernelEvent::Network(e) => e.observed_at,
            KernelEvent::Capability(e) => e.observed_at,
            KernelEvent::BpfLoad(e) => e.observed_at,
            KernelEvent::Kprobe(e) => e.observed_at,
            KernelEvent::Uprobe(e) => e.observed_at,
        }
    }

    /// Process responsible for the event (may be `None` for kernel-thread
    /// originating events).
    pub fn process(&self) -> Option<&Process> {
        match self {
            KernelEvent::ProcessExec(e) => Some(&e.process),
            KernelEvent::ProcessExit(e) => Some(&e.process),
            KernelEvent::FileOp(e) => Some(&e.process),
            KernelEvent::Network(e) => Some(&e.process),
            KernelEvent::Capability(e) => Some(&e.process),
            KernelEvent::BpfLoad(e) => Some(&e.process),
            KernelEvent::Kprobe(e) => e.process.as_ref(),
            KernelEvent::Uprobe(e) => e.process.as_ref(),
        }
    }

    /// A short kind-tag used in logs + metrics.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            KernelEvent::ProcessExec(_) => "process_exec",
            KernelEvent::ProcessExit(_) => "process_exit",
            KernelEvent::FileOp(_) => "file_op",
            KernelEvent::Network(_) => "network",
            KernelEvent::Capability(_) => "capability",
            KernelEvent::BpfLoad(_) => "bpf_load",
            KernelEvent::Kprobe(_) => "kprobe",
            KernelEvent::Uprobe(_) => "uprobe",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::process_exec::ProcessExecEvent;
    use crate::process::{Credentials, Namespaces, Process};
    use chrono::TimeZone;

    fn ts(s: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(s, 0).unwrap()
    }

    fn make_process() -> Process {
        Process {
            exec_id: "x".into(),
            pid: 100,
            pid_in_ns: 1,
            binary: "/bin/sh".into(),
            arguments: String::new(),
            cwd: "/".into(),
            credentials: Credentials::default(),
            namespaces: Namespaces::default(),
            parent_exec_id: None,
            container_id: None,
            pod_name: None,
            pod_namespace: None,
            start_time: ts(0),
            end_time: None,
        }
    }

    #[test]
    fn test_kind_tag_for_each_variant() {
        let p = make_process();
        let ev = KernelEvent::ProcessExec(ProcessExecEvent {
            process: p.clone(),
            ancestors: vec![],
            observed_at: ts(1),
        });
        assert_eq!(ev.kind_tag(), "process_exec");
        assert_eq!(ev.observed_at(), ts(1));
        assert!(ev.process().is_some());
    }
}

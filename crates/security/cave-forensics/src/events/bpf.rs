// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! eBPF program load events.
//!
//! Upstream: `pkg/grpc/tracing/kprobe.go` `BPF_PROG_LOAD` standard-library
//! probe + `pkg/sensors/tracing/genericbpf.go`.

use crate::process::Process;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BpfProgType {
    SocketFilter,
    Kprobe,
    Tracepoint,
    Xdp,
    PerfEvent,
    CgroupSkb,
    LircMode2,
    SkSkb,
    CgroupSock,
    CgroupDevice,
    SkMsg,
    RawTracepoint,
    CgroupSockopt,
    LsmCgroup,
    Lsm,
    Unspec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BpfLoadEvent {
    pub prog_type: BpfProgType,
    pub prog_name: String,
    pub prog_id: u32,
    pub insn_count: u32,
    pub tag: String,
    pub license: String,
    pub process: Process,
    pub observed_at: DateTime<Utc>,
}

impl BpfLoadEvent {
    /// True if the program is in a privileged category (LSM, Lsm-cgroup
    /// or raw_tracepoint). Tetragon flags these heavily.
    pub fn is_privileged_type(&self) -> bool {
        matches!(
            self.prog_type,
            BpfProgType::Lsm
                | BpfProgType::LsmCgroup
                | BpfProgType::RawTracepoint
                | BpfProgType::Kprobe
                | BpfProgType::Tracepoint
        )
    }

    /// True if the program license string is GPL-compatible (required for
    /// many BPF helper functions).
    pub fn is_gpl(&self) -> bool {
        matches!(
            self.license.as_str(),
            "GPL" | "GPL v2" | "Dual BSD/GPL" | "Dual MIT/GPL" | "Dual MPL/GPL"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Credentials, Namespaces};
    use chrono::TimeZone;

    fn ev(prog_type: BpfProgType, license: &str) -> BpfLoadEvent {
        BpfLoadEvent {
            prog_type,
            prog_name: "test".into(),
            prog_id: 1,
            insn_count: 100,
            tag: "deadbeef".into(),
            license: license.into(),
            process: Process {
                exec_id: "x".into(),
                pid: 1,
                pid_in_ns: 1,
                binary: "/usr/sbin/bpftool".into(),
                arguments: String::new(),
                cwd: "/".into(),
                credentials: Credentials::default(),
                namespaces: Namespaces::default(),
                parent_exec_id: None,
                container_id: None,
                pod_name: None,
                pod_namespace: None,
                start_time: Utc.timestamp_opt(0, 0).unwrap(),
                end_time: None,
            },
            observed_at: Utc.timestamp_opt(0, 0).unwrap(),
        }
    }

    #[test]
    fn test_privileged_kinds() {
        assert!(ev(BpfProgType::Lsm, "GPL").is_privileged_type());
        assert!(ev(BpfProgType::Kprobe, "GPL").is_privileged_type());
        assert!(!ev(BpfProgType::Xdp, "GPL").is_privileged_type());
    }

    #[test]
    fn test_gpl_compat() {
        assert!(ev(BpfProgType::Xdp, "GPL").is_gpl());
        assert!(ev(BpfProgType::Xdp, "Dual BSD/GPL").is_gpl());
        assert!(!ev(BpfProgType::Xdp, "Proprietary").is_gpl());
    }

    #[test]
    fn test_serde_roundtrip() {
        let e = ev(BpfProgType::Lsm, "GPL");
        let j = serde_json::to_string(&e).unwrap();
        let back: BpfLoadEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn test_prog_type_serde_snake_case() {
        let j = serde_json::to_string(&BpfProgType::LsmCgroup).unwrap();
        assert_eq!(j, "\"lsm_cgroup\"");
    }
}

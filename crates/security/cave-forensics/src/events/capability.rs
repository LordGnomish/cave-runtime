// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Capability-check kernel events.
//!
//! Upstream: `pkg/grpc/tracing/kprobe.go` capability standard-library
//! probes (`security_capable`, `security_bprm_committing_creds`).

use crate::process::Process;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityCheckEvent {
    pub capability_bit: u64,
    pub capability_name: String,
    pub allowed: bool,
    pub process: Process,
    pub observed_at: DateTime<Utc>,
}

impl CapabilityCheckEvent {
    /// True if a typically privileged capability was used.
    pub fn is_dangerous(&self) -> bool {
        use crate::process::cap::*;
        let dangerous_mask = CAP_SYS_ADMIN
            | CAP_SYS_MODULE
            | CAP_SYS_PTRACE
            | CAP_SYS_BOOT
            | CAP_DAC_OVERRIDE
            | CAP_DAC_READ_SEARCH
            | CAP_BPF;
        self.capability_bit & dangerous_mask != 0
    }
}

/// Look up the symbolic name for a capability bit (matches the
/// upstream `pkg/reader/caps/caps.go` lookup table).
pub fn capability_name(bit: u64) -> &'static str {
    use crate::process::cap::*;
    match bit {
        b if b == CAP_CHOWN => "CAP_CHOWN",
        b if b == CAP_DAC_OVERRIDE => "CAP_DAC_OVERRIDE",
        b if b == CAP_DAC_READ_SEARCH => "CAP_DAC_READ_SEARCH",
        b if b == CAP_FOWNER => "CAP_FOWNER",
        b if b == CAP_KILL => "CAP_KILL",
        b if b == CAP_SETGID => "CAP_SETGID",
        b if b == CAP_SETUID => "CAP_SETUID",
        b if b == CAP_NET_BIND_SERVICE => "CAP_NET_BIND_SERVICE",
        b if b == CAP_NET_ADMIN => "CAP_NET_ADMIN",
        b if b == CAP_NET_RAW => "CAP_NET_RAW",
        b if b == CAP_SYS_MODULE => "CAP_SYS_MODULE",
        b if b == CAP_SYS_ADMIN => "CAP_SYS_ADMIN",
        b if b == CAP_SYS_BOOT => "CAP_SYS_BOOT",
        b if b == CAP_SYS_PTRACE => "CAP_SYS_PTRACE",
        b if b == CAP_BPF => "CAP_BPF",
        b if b == CAP_PERFMON => "CAP_PERFMON",
        _ => "CAP_UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{cap, Credentials, Namespaces};
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    fn proc() -> Process {
        Process {
            exec_id: "x".into(),
            pid: 1,
            pid_in_ns: 1,
            binary: "/usr/bin/nmap".into(),
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

    fn ev(bit: u64) -> CapabilityCheckEvent {
        CapabilityCheckEvent {
            capability_bit: bit,
            capability_name: capability_name(bit).to_string(),
            allowed: true,
            process: proc(),
            observed_at: ts(),
        }
    }

    #[test]
    fn test_dangerous_caps_flagged() {
        for c in [
            cap::CAP_SYS_ADMIN,
            cap::CAP_SYS_MODULE,
            cap::CAP_BPF,
            cap::CAP_DAC_OVERRIDE,
        ] {
            assert!(ev(c).is_dangerous(), "{:#x} should be dangerous", c);
        }
    }

    #[test]
    fn test_benign_caps_not_dangerous() {
        for c in [cap::CAP_CHOWN, cap::CAP_NET_BIND_SERVICE, cap::CAP_FOWNER] {
            assert!(!ev(c).is_dangerous());
        }
    }

    #[test]
    fn test_capability_name_resolution() {
        assert_eq!(capability_name(cap::CAP_SYS_ADMIN), "CAP_SYS_ADMIN");
        assert_eq!(capability_name(cap::CAP_BPF), "CAP_BPF");
        assert_eq!(capability_name(0x123456789abcdef0), "CAP_UNKNOWN");
    }

    #[test]
    fn test_event_serde_roundtrip() {
        let e = ev(cap::CAP_NET_RAW);
        let j = serde_json::to_string(&e).unwrap();
        let back: CapabilityCheckEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }
}

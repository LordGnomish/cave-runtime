// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic kprobe + uprobe registration metadata events.
//!
//! Upstream: `pkg/sensors/tracing/genericKprobeSensor.go`,
//! `pkg/sensors/tracing/genericUprobeSensor.go`.

use crate::process::Process;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KprobeArg {
    pub index: u32,
    pub r#type: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KprobeEvent {
    pub policy_name: String,
    pub function_name: String,
    pub args: Vec<KprobeArg>,
    pub return_value: Option<serde_json::Value>,
    pub process: Option<Process>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UprobeEvent {
    pub policy_name: String,
    pub binary_path: String,
    pub symbol: String,
    pub args: Vec<KprobeArg>,
    pub process: Option<Process>,
    pub observed_at: DateTime<Utc>,
}

impl KprobeEvent {
    /// True if this fired against a syscall (function name starts with
    /// `sys_` or `__x64_sys_`).
    pub fn is_syscall(&self) -> bool {
        self.function_name.starts_with("sys_") || self.function_name.starts_with("__x64_sys_")
    }
}

impl UprobeEvent {
    /// True if the binary is in a typical secret-handling library set.
    pub fn touches_crypto_lib(&self) -> bool {
        const NEEDLES: &[&str] = &["libssl", "libcrypto", "libgnutls", "libnss"];
        NEEDLES.iter().any(|n| self.binary_path.contains(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    #[test]
    fn test_kprobe_is_syscall_sys_prefix() {
        let k = KprobeEvent {
            policy_name: "p".into(),
            function_name: "sys_openat".into(),
            args: vec![],
            return_value: None,
            process: None,
            observed_at: ts(),
        };
        assert!(k.is_syscall());
    }

    #[test]
    fn test_kprobe_is_syscall_x64_prefix() {
        let k = KprobeEvent {
            policy_name: "p".into(),
            function_name: "__x64_sys_execve".into(),
            args: vec![],
            return_value: None,
            process: None,
            observed_at: ts(),
        };
        assert!(k.is_syscall());
    }

    #[test]
    fn test_kprobe_is_not_syscall_for_internal_fn() {
        let k = KprobeEvent {
            policy_name: "p".into(),
            function_name: "do_filp_open".into(),
            args: vec![],
            return_value: None,
            process: None,
            observed_at: ts(),
        };
        assert!(!k.is_syscall());
    }

    #[test]
    fn test_uprobe_crypto_lib_detected() {
        for lib in ["/usr/lib/libssl.so.3", "/usr/lib/libcrypto.so", "/lib/x86_64-linux-gnu/libnss_files.so"] {
            let u = UprobeEvent {
                policy_name: "p".into(),
                binary_path: lib.into(),
                symbol: "f".into(),
                args: vec![],
                process: None,
                observed_at: ts(),
            };
            assert!(u.touches_crypto_lib());
        }
    }

    #[test]
    fn test_uprobe_non_crypto_lib() {
        let u = UprobeEvent {
            policy_name: "p".into(),
            binary_path: "/bin/cat".into(),
            symbol: "main".into(),
            args: vec![],
            process: None,
            observed_at: ts(),
        };
        assert!(!u.touches_crypto_lib());
    }

    #[test]
    fn test_kprobe_arg_serde() {
        let a = KprobeArg {
            index: 0,
            r#type: "string".into(),
            value: serde_json::json!("/etc/hosts"),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: KprobeArg = serde_json::from_str(&j).unwrap();
        assert_eq!(back, a);
    }
}

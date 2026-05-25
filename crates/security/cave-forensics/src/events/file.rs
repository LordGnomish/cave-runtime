// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! File-related kernel events.
//!
//! Upstream: `pkg/grpc/tracing/kprobe.go` + the standard `file_open`/`file_*`
//! kprobe set + `pkg/tracingpolicy/standardlib/file.go`.

use crate::process::Process;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileOp {
    Open,
    Read,
    Write,
    Unlink,
    Rename,
    Chmod,
    Chown,
    Mmap,
    Truncate,
}

/// Pure-Rust wrapper around the Linux `open(2)` flag bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OpenFlags(pub u32);

impl OpenFlags {
    pub const O_RDONLY: OpenFlags = OpenFlags(0o0);
    pub const O_WRONLY: OpenFlags = OpenFlags(0o1);
    pub const O_RDWR: OpenFlags = OpenFlags(0o2);
    pub const O_CREAT: OpenFlags = OpenFlags(0o100);
    pub const O_EXCL: OpenFlags = OpenFlags(0o200);
    pub const O_TRUNC: OpenFlags = OpenFlags(0o1000);
    pub const O_APPEND: OpenFlags = OpenFlags(0o2000);
    pub const O_NONBLOCK: OpenFlags = OpenFlags(0o4000);
    pub const O_CLOEXEC: OpenFlags = OpenFlags(0o2000000);

    pub fn from_bits_truncate(bits: u32) -> Self {
        OpenFlags(bits)
    }

    pub fn bits(self) -> u32 {
        self.0
    }

    pub fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }
}

impl std::ops::BitOr for OpenFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        OpenFlags(self.0 | rhs.0)
    }
}

/// A file-system kernel event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEvent {
    pub op: FileOp,
    pub path: String,
    #[serde(default)]
    pub flags: Option<u32>,
    #[serde(default)]
    pub mode: Option<u32>,
    pub process: Process,
    pub observed_at: DateTime<Utc>,
}

impl FileEvent {
    /// True if the file op opens the file for writing.
    pub fn is_write(&self) -> bool {
        matches!(
            self.op,
            FileOp::Write | FileOp::Truncate | FileOp::Chmod | FileOp::Chown | FileOp::Unlink | FileOp::Rename
        ) || self.flags.is_some_and(|f| OpenFlags::from_bits_truncate(f).intersects(OpenFlags::O_WRONLY | OpenFlags::O_RDWR | OpenFlags::O_TRUNC))
    }

    /// True if path is inside `/proc` (`/proc/self/...`, `/proc/<pid>/...`).
    pub fn is_proc_access(&self) -> bool {
        self.path.starts_with("/proc/")
    }

    /// True if path is a typical sensitive secret (`/etc/shadow`,
    /// `/etc/sudoers`, `/root/.ssh/...`, container-runtime sockets).
    pub fn is_sensitive(&self) -> bool {
        const NEEDLES: &[&str] = &[
            "/etc/shadow",
            "/etc/sudoers",
            "/root/.ssh",
            "/var/run/docker.sock",
            "/var/run/containerd/containerd.sock",
            "/run/cri-runtime",
            "/var/run/secrets/kubernetes.io",
        ];
        NEEDLES.iter().any(|n| self.path.starts_with(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Credentials, Namespaces};
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    fn proc() -> Process {
        Process {
            exec_id: "x".into(),
            pid: 1,
            pid_in_ns: 1,
            binary: "/bin/cat".into(),
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

    fn ev(op: FileOp, path: &str) -> FileEvent {
        FileEvent {
            op,
            path: path.into(),
            flags: None,
            mode: None,
            process: proc(),
            observed_at: ts(),
        }
    }

    #[test]
    fn test_unlink_is_write() {
        assert!(ev(FileOp::Unlink, "/tmp/x").is_write());
    }

    #[test]
    fn test_open_flags_classifies_as_write() {
        let mut e = ev(FileOp::Open, "/tmp/x");
        e.flags = Some((OpenFlags::O_WRONLY | OpenFlags::O_CREAT).bits());
        assert!(e.is_write());
    }

    #[test]
    fn test_open_readonly_is_not_write() {
        let mut e = ev(FileOp::Open, "/etc/hosts");
        e.flags = Some(OpenFlags::O_RDONLY.bits());
        assert!(!e.is_write());
    }

    #[test]
    fn test_proc_access_detected() {
        assert!(ev(FileOp::Read, "/proc/self/maps").is_proc_access());
        assert!(!ev(FileOp::Read, "/etc/hosts").is_proc_access());
    }

    #[test]
    fn test_sensitive_paths_detected() {
        for p in ["/etc/shadow", "/etc/sudoers.d/x", "/root/.ssh/id_rsa"] {
            assert!(ev(FileOp::Read, p).is_sensitive(), "{} should be sensitive", p);
        }
        assert!(!ev(FileOp::Read, "/tmp/random").is_sensitive());
        assert!(ev(FileOp::Read, "/var/run/docker.sock").is_sensitive());
    }

    #[test]
    fn test_file_event_serde_roundtrip() {
        let e = ev(FileOp::Write, "/etc/foo");
        let j = serde_json::to_string(&e).unwrap();
        let back: FileEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn test_file_op_serde_snake_case() {
        let j = serde_json::to_string(&FileOp::Mmap).unwrap();
        assert_eq!(j, "\"mmap\"");
    }

    #[test]
    fn test_open_flags_bits_match_linux() {
        assert_eq!(OpenFlags::O_RDONLY.bits(), 0o0);
        assert_eq!(OpenFlags::O_WRONLY.bits(), 0o1);
        assert_eq!(OpenFlags::O_RDWR.bits(), 0o2);
        assert_eq!(OpenFlags::O_CREAT.bits(), 0o100);
        assert_eq!(OpenFlags::O_TRUNC.bits(), 0o1000);
        assert_eq!(OpenFlags::O_APPEND.bits(), 0o2000);
    }
}

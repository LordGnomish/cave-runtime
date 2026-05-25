// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: Sentry config schema adapted from google/gvisor pkg/sentry/* (Apache-2.0).
//! gVisor Sentry — user-space kernel.
//!
//! This module models Sentry's config shape (platform, network, fs) and the
//! syscall allowlist surface used by `pkg/seccomp/`. The actual ptrace /
//! KVM / systrap kernel substitution is OUT OF SCOPE (no-FFI policy).

use serde::{Deserialize, Serialize};

/// Sentry execution platform — `runsc/cli/main.go --platform=…`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SentryPlatform {
    /// `ptrace` — POSIX ptrace-based syscall interception. Default.
    Ptrace,
    /// `kvm` — hardware-virtualized; uses /dev/kvm.
    Kvm,
    /// `systrap` — seccomp-bpf based syscall interception (newer).
    Systrap,
}

impl SentryPlatform {
    pub fn as_flag(self) -> &'static str {
        match self {
            SentryPlatform::Ptrace => "ptrace",
            SentryPlatform::Kvm => "kvm",
            SentryPlatform::Systrap => "systrap",
        }
    }
}

/// Network stack — `--network=sandbox|host|none`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SentryNetwork {
    /// `sandbox` — netstack (gVisor's userspace network stack). Default.
    Sandbox,
    /// `host` — share host's network namespace. Less isolated.
    Host,
    /// `none` — no networking.
    None,
}

/// Filesystem mode — `--overlay`, `--overlay2`, `--file-access=…`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SentryFs {
    /// Direct gofer 9P access.
    Direct,
    /// Overlay over gofer (default).
    Overlay,
    /// Per-mount overlay (`--overlay2`).
    Overlay2,
}

/// Top-level Sentry config — `runsc/config/config.go::Config`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SentryConfig {
    pub platform: SentryPlatform,
    pub network: SentryNetwork,
    pub fs: SentryFs,
    pub debug: bool,
    pub strace: bool,
    /// Compatibility/diagnostics: numeric watchdog action — `--watchdog-action`.
    pub watchdog_action: WatchdogAction,
    /// Per-container seccomp filter — applied to the *Sentry* itself.
    pub host_seccomp: bool,
    /// Trace tag — for cilium-style flow records.
    pub trace_tag: Option<String>,
}

impl Default for SentryConfig {
    fn default() -> Self {
        SentryConfig {
            platform: SentryPlatform::Ptrace,
            network: SentryNetwork::Sandbox,
            fs: SentryFs::Overlay,
            debug: false,
            strace: false,
            watchdog_action: WatchdogAction::LogWarning,
            host_seccomp: true,
            trace_tag: None,
        }
    }
}

/// `--watchdog-action`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WatchdogAction {
    LogWarning,
    Panic,
}

/// Syscall allowlist entry — `pkg/seccomp/seccomp.go::RuleSet`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyscallRule {
    /// e.g. `read`, `write`, `openat`.
    pub name: String,
    /// `allow`, `deny`, `trap`, `kill`, `errno:<n>`.
    pub action: String,
}

impl SyscallRule {
    pub fn allow(name: impl Into<String>) -> Self {
        SyscallRule { name: name.into(), action: "allow".into() }
    }
    pub fn deny(name: impl Into<String>) -> Self {
        SyscallRule { name: name.into(), action: "deny".into() }
    }
}

/// Default Sentry syscall filter — a tiny taste of upstream's `~330` allowed.
pub fn default_allowlist() -> Vec<SyscallRule> {
    [
        "read", "write", "openat", "close", "stat", "fstat", "lstat",
        "mmap", "mprotect", "munmap", "brk", "rt_sigaction", "rt_sigprocmask",
        "ioctl", "pread64", "pwrite64", "readv", "writev", "access", "pipe",
        "select", "sched_yield", "mremap", "msync", "mincore", "madvise",
        "dup", "dup2", "pause", "nanosleep", "getitimer", "alarm", "setitimer",
        "getpid", "sendfile", "socket", "connect", "accept", "sendto", "recvfrom",
        "shutdown", "bind", "listen", "getsockname", "getpeername", "setsockopt",
        "getsockopt", "clone", "fork", "vfork", "execve", "exit", "wait4", "kill",
    ]
    .into_iter()
    .map(SyscallRule::allow)
    .collect()
}

/// Capability filter — applied by Sentry when masking host caps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CapabilityFilter {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl CapabilityFilter {
    pub fn permits(&self, cap: &str) -> bool {
        if self.deny.iter().any(|c| c == cap) {
            return false;
        }
        if self.allow.is_empty() {
            return true;
        }
        self.allow.iter().any(|c| c == cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_flag_strings() {
        assert_eq!(SentryPlatform::Ptrace.as_flag(), "ptrace");
        assert_eq!(SentryPlatform::Kvm.as_flag(), "kvm");
        assert_eq!(SentryPlatform::Systrap.as_flag(), "systrap");
    }

    #[test]
    fn default_config_safe() {
        let c = SentryConfig::default();
        assert_eq!(c.platform, SentryPlatform::Ptrace);
        assert_eq!(c.network, SentryNetwork::Sandbox);
        assert!(c.host_seccomp);
    }

    #[test]
    fn default_allowlist_has_50_plus() {
        let list = default_allowlist();
        assert!(list.len() >= 50, "got {}", list.len());
        assert!(list.iter().any(|r| r.name == "read"));
        assert!(list.iter().any(|r| r.name == "execve"));
    }

    #[test]
    fn capability_filter_allows_all_by_default() {
        let f = CapabilityFilter::default();
        assert!(f.permits("CAP_KILL"));
        assert!(f.permits("CAP_SYS_ADMIN"));
    }

    #[test]
    fn capability_filter_explicit_deny() {
        let f = CapabilityFilter {
            allow: vec![],
            deny: vec!["CAP_SYS_ADMIN".into()],
        };
        assert!(!f.permits("CAP_SYS_ADMIN"));
        assert!(f.permits("CAP_KILL"));
    }

    #[test]
    fn capability_filter_allowlist_mode() {
        let f = CapabilityFilter {
            allow: vec!["CAP_KILL".into()],
            deny: vec![],
        };
        assert!(f.permits("CAP_KILL"));
        assert!(!f.permits("CAP_SYS_ADMIN"));
    }

    #[test]
    fn rule_constructors() {
        let r = SyscallRule::allow("read");
        assert_eq!(r.action, "allow");
        let r = SyscallRule::deny("ptrace");
        assert_eq!(r.action, "deny");
    }

    #[test]
    fn fs_modes_roundtrip() {
        let c = SentryConfig { fs: SentryFs::Overlay2, ..SentryConfig::default() };
        let j = serde_json::to_string(&c).unwrap();
        let back: SentryConfig = serde_json::from_str(&j).unwrap();
        assert_eq!(back.fs, SentryFs::Overlay2);
    }
}

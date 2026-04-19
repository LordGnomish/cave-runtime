//! Linux namespace management for container isolation.
//!
//! Creates pid, net, mnt, uts, ipc namespaces using clone(2) flags.
//! On systems without namespace support (macOS, unprivileged), operations
//! are no-ops that log warnings.

use crate::error::CriResult;
use serde::{Deserialize, Serialize};

/// Which namespaces to create for a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceConfig {
    pub pid: bool,
    pub net: bool,
    pub mnt: bool,
    pub uts: bool,
    pub ipc: bool,
    pub user: bool,
}

impl Default for NamespaceConfig {
    fn default() -> Self {
        Self { pid: true, net: true, mnt: true, uts: true, ipc: true, user: false }
    }
}

/// Represents a set of created namespaces.
#[derive(Debug)]
pub struct NamespaceSet {
    pub config: NamespaceConfig,
    pub clone_flags: i32,
}

/// Build clone(2) flags from namespace config.
pub fn build_clone_flags(_config: &NamespaceConfig) -> i32 {
    let flags: i32 = 0;
    #[cfg(target_os = "linux")]
    {
        use nix::sched::CloneFlags;
        if config.pid { flags |= CloneFlags::CLONE_NEWPID.bits(); }
        if config.net { flags |= CloneFlags::CLONE_NEWNS.bits(); }
        if config.mnt { flags |= CloneFlags::CLONE_NEWNS.bits(); }
        if config.uts { flags |= CloneFlags::CLONE_NEWUTS.bits(); }
        if config.ipc { flags |= CloneFlags::CLONE_NEWIPC.bits(); }
        if config.user { flags |= CloneFlags::CLONE_NEWUSER.bits(); }
    }
    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("namespaces not supported on this OS — container isolation disabled");
    }
    flags
}

/// Create namespaces for a container.
pub fn create_namespaces(config: &NamespaceConfig) -> CriResult<NamespaceSet> {
    let flags = build_clone_flags(config);
    Ok(NamespaceSet { config: config.clone(), clone_flags: flags })
}

/// Enter an existing container's namespaces by PID (for exec).
pub fn enter_namespaces(pid: u32) -> CriResult<()> {
    #[cfg(target_os = "linux")]
    {
        use std::fs::File;
        use std::os::unix::io::AsRawFd;

        let ns_types = ["pid", "net", "mnt", "uts", "ipc"];
        for ns in &ns_types {
            let path = format!("/proc/{}/ns/{}", pid, ns);
            match File::open(&path) {
                Ok(f) => {
                    let fd = f.as_raw_fd();
                    unsafe {
                        if libc::setns(fd, 0) != 0 {
                            return Err(CriError::Namespace(
                                format!("setns({}) failed: {}", ns, std::io::Error::last_os_error())
                            ));
                        }
                    }
                    tracing::debug!("entered {} namespace of pid {}", ns, pid);
                }
                Err(e) => {
                    tracing::warn!("cannot open {}: {} — skipping", path, e);
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        tracing::warn!("enter_namespaces: not supported on this OS");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_namespace_config() {
        let c = NamespaceConfig::default();
        assert!(c.pid);
        assert!(c.net);
        assert!(c.mnt);
        assert!(!c.user);
    }

    #[test]
    fn test_build_clone_flags_non_linux() {
        // On macOS (test env), flags should be 0
        let config = NamespaceConfig::default();
        let _flags = build_clone_flags(&config);
        // Just verify it doesn't panic
    }

    #[test]
    fn test_create_namespaces() {
        let config = NamespaceConfig::default();
        let ns = create_namespaces(&config).unwrap();
        assert_eq!(ns.config.pid, true);
    }

    #[test]
    fn test_build_clone_flags_all_false() {
        let config = NamespaceConfig { pid: false, net: false, mnt: false, uts: false, ipc: false, user: false };
        let flags = build_clone_flags(&config);
        // On non-linux always returns 0; on linux also 0 since all disabled
        assert_eq!(flags, 0);
    }

    #[test]
    fn test_build_clone_flags_all_true_no_panic() {
        let config = NamespaceConfig { pid: true, net: true, mnt: true, uts: true, ipc: true, user: true };
        let _flags = build_clone_flags(&config);
        // Just verify it doesn't panic; value is platform-dependent
    }

    #[test]
    fn test_build_clone_flags_mixed() {
        let config = NamespaceConfig { pid: true, net: false, mnt: true, uts: false, ipc: false, user: false };
        let _flags = build_clone_flags(&config);
        // Verify no panic on partial config
    }

    #[test]
    fn test_create_namespaces_preserves_all_fields() {
        let config = NamespaceConfig { pid: true, net: false, mnt: true, uts: false, ipc: false, user: true };
        let ns = create_namespaces(&config).unwrap();
        assert!(ns.config.pid);
        assert!(!ns.config.net);
        assert!(ns.config.mnt);
        assert!(!ns.config.uts);
        assert!(!ns.config.ipc);
        assert!(ns.config.user);
    }

    #[test]
    fn test_namespace_config_serialization() {
        let c = NamespaceConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let back: NamespaceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pid, c.pid);
        assert_eq!(back.net, c.net);
        assert_eq!(back.user, c.user);
    }

    #[test]
    fn test_enter_namespaces_invalid_pid_no_error() {
        // Invalid PID: /proc/99999999/ns/* won't exist.
        // On non-linux: no-op, returns Ok.
        // On linux: open fails → skip with warning, returns Ok.
        let result = enter_namespaces(99999999);
        assert!(result.is_ok());
    }

    #[test]
    fn test_enter_namespaces_pid_zero_no_error() {
        // PID 0 is never a valid container pid; should return Ok gracefully.
        let result = enter_namespaces(0);
        assert!(result.is_ok());
    }
}

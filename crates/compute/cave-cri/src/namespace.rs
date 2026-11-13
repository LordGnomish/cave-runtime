// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
        Self {
            pid: true,
            net: true,
            mnt: true,
            uts: true,
            ipc: true,
            user: false,
        }
    }
}

/// Represents a set of created namespaces.
#[derive(Debug)]
pub struct NamespaceSet {
    pub config: NamespaceConfig,
    pub clone_flags: i32,
}

/// Build clone(2) flags from namespace config.
pub fn build_clone_flags(config: &NamespaceConfig) -> i32 {
    #[cfg(target_os = "linux")]
    {
        use nix::sched::CloneFlags;
        let mut flags: i32 = 0;
        if config.pid {
            flags |= CloneFlags::CLONE_NEWPID.bits();
        }
        if config.net {
            flags |= CloneFlags::CLONE_NEWNET.bits();
        }
        if config.mnt {
            flags |= CloneFlags::CLONE_NEWNS.bits();
        }
        if config.uts {
            flags |= CloneFlags::CLONE_NEWUTS.bits();
        }
        if config.ipc {
            flags |= CloneFlags::CLONE_NEWIPC.bits();
        }
        if config.user {
            flags |= CloneFlags::CLONE_NEWUSER.bits();
        }
        flags
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        tracing::warn!("namespaces not supported on this OS — container isolation disabled");
        0
    }
}

/// Create namespaces for a container.
pub fn create_namespaces(config: &NamespaceConfig) -> CriResult<NamespaceSet> {
    let flags = build_clone_flags(config);
    Ok(NamespaceSet {
        config: config.clone(),
        clone_flags: flags,
    })
}

/// Enter an existing container's namespaces by PID (for exec).
pub fn enter_namespaces(pid: u32) -> CriResult<()> {
    #[cfg(target_os = "linux")]
    {
        use crate::error::CriError;
        use nix::sched::{setns, CloneFlags};
        use std::fs::File;

        let ns_types = ["pid", "net", "mnt", "uts", "ipc"];
        for ns in &ns_types {
            let path = format!("/proc/{}/ns/{}", pid, ns);
            match File::open(&path) {
                Ok(f) => {
                    // An empty flag set is setns(2)'s nstype 0: the fd may
                    // refer to a namespace of any type.
                    setns(&f, CloneFlags::empty())
                        .map_err(|e| CriError::Namespace(format!("setns({}) failed: {}", ns, e)))?;
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
        let config = NamespaceConfig {
            pid: false,
            net: false,
            mnt: false,
            uts: false,
            ipc: false,
            user: false,
        };
        let flags = build_clone_flags(&config);
        // On non-linux always returns 0; on linux also 0 since all disabled
        assert_eq!(flags, 0);
    }

    #[test]
    fn test_build_clone_flags_all_true_no_panic() {
        let config = NamespaceConfig {
            pid: true,
            net: true,
            mnt: true,
            uts: true,
            ipc: true,
            user: true,
        };
        let _flags = build_clone_flags(&config);
        // Just verify it doesn't panic; value is platform-dependent
    }

    #[test]
    fn test_build_clone_flags_mixed() {
        let config = NamespaceConfig {
            pid: true,
            net: false,
            mnt: true,
            uts: false,
            ipc: false,
            user: false,
        };
        let _flags = build_clone_flags(&config);
        // Verify no panic on partial config
    }

    #[test]
    fn test_create_namespaces_preserves_all_fields() {
        let config = NamespaceConfig {
            pid: true,
            net: false,
            mnt: true,
            uts: false,
            ipc: false,
            user: true,
        };
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

    /// `open_foreign_namespaces` against a fake `/proc`: regular files stand
    /// in for the namespace entries. Two entries are "the same namespace"
    /// when they are the same file (a hard link has the same device and
    /// inode number, which is the comparison namespaces(7) prescribes), so
    /// this needs no privileges and no second process.
    #[cfg(target_os = "linux")]
    mod foreign_namespaces {
        use super::super::*;
        use nix::errno::Errno;
        use nix::sched::CloneFlags;
        use std::os::unix::fs::MetadataExt;
        use std::path::{Path, PathBuf};

        const TARGET_PID: u32 = 4242;
        const KINDS: [&str; 5] = ["pid", "net", "mnt", "uts", "ipc"];

        fn ns_dir(proc_root: &Path, owner: &str) -> PathBuf {
            let dir = proc_root.join(owner).join("ns");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        /// A fake `/proc` in which the caller has all five namespaces and
        /// the target has an entry for each kind in `shared` (the caller's
        /// namespace) and in `foreign` (a namespace of its own).
        fn fake_proc(shared: &[&str], foreign: &[&str]) -> tempfile::TempDir {
            let root = tempfile::tempdir().unwrap();
            let own = ns_dir(root.path(), "thread-self");
            let target = ns_dir(root.path(), &TARGET_PID.to_string());
            for kind in KINDS {
                std::fs::write(own.join(kind), b"").unwrap();
            }
            for kind in shared {
                std::fs::hard_link(own.join(kind), target.join(kind)).unwrap();
            }
            for kind in foreign {
                std::fs::write(target.join(kind), b"").unwrap();
            }
            root
        }

        fn names(handles: &[NamespaceHandle]) -> Vec<&'static str> {
            handles.iter().map(|h| h.name).collect()
        }

        #[test]
        fn namespaces_shared_with_the_caller_are_left_out() {
            let root = fake_proc(&KINDS, &[]);

            let handles = open_foreign_namespaces(root.path(), TARGET_PID);

            assert!(handles.is_empty(), "got {:?}", names(&handles));
        }

        #[test]
        fn foreign_namespaces_are_opened_with_their_kernel_type() {
            let root = fake_proc(&["pid", "mnt", "ipc"], &["net", "uts"]);

            let handles = open_foreign_namespaces(root.path(), TARGET_PID);

            assert_eq!(names(&handles), ["net", "uts"]);
            assert_eq!(handles[0].kind, CloneFlags::CLONE_NEWNET);
            assert_eq!(handles[1].kind, CloneFlags::CLONE_NEWUTS);

            // The descriptor is the target's entry, not the caller's.
            let target_net = root.path().join("4242/ns/net");
            let expected = std::fs::metadata(target_net).unwrap();
            let opened = handles[0].file.metadata().unwrap();
            assert_eq!(
                (opened.dev(), opened.ino()),
                (expected.dev(), expected.ino())
            );
        }

        #[test]
        fn every_kind_is_covered_in_a_fixed_join_order() {
            let root = fake_proc(&[], &KINDS);

            let handles = open_foreign_namespaces(root.path(), TARGET_PID);

            assert_eq!(names(&handles), ["pid", "net", "mnt", "uts", "ipc"]);
            let kinds: Vec<CloneFlags> = handles.iter().map(|h| h.kind).collect();
            assert_eq!(
                kinds,
                [
                    CloneFlags::CLONE_NEWPID,
                    CloneFlags::CLONE_NEWNET,
                    CloneFlags::CLONE_NEWNS,
                    CloneFlags::CLONE_NEWUTS,
                    CloneFlags::CLONE_NEWIPC,
                ]
            );
        }

        #[test]
        fn entries_that_cannot_be_opened_are_skipped() {
            let root = fake_proc(&[], &["mnt"]);

            let handles = open_foreign_namespaces(root.path(), TARGET_PID);
            assert_eq!(names(&handles), ["mnt"]);

            let gone = open_foreign_namespaces(root.path(), TARGET_PID + 1);
            assert!(gone.is_empty(), "got {:?}", names(&gone));
        }

        /// If the caller's own entry cannot be read there is no proof that
        /// the namespace is shared, so it must still be joined.
        #[test]
        fn a_namespace_counts_as_foreign_when_the_callers_own_is_unreadable() {
            let root = fake_proc(&[], &["net"]);
            std::fs::remove_file(root.path().join("thread-self/ns/net")).unwrap();

            let handles = open_foreign_namespaces(root.path(), TARGET_PID);

            assert_eq!(names(&handles), ["net"]);
        }

        #[test]
        fn the_real_proc_reports_no_foreign_namespace_for_this_process() {
            for kind in KINDS {
                let entry = format!("/proc/thread-self/ns/{kind}");
                assert!(
                    std::fs::metadata(&entry).is_ok(),
                    "{entry} must be readable, or nothing is compared"
                );
            }

            let handles = open_foreign_namespaces(Path::new("/proc"), std::process::id());

            assert!(handles.is_empty(), "got {:?}", names(&handles));
        }

        #[test]
        fn joining_nothing_succeeds() {
            assert_eq!(join_namespaces(&[]), Ok(()));
        }

        /// setns(2) rejects a descriptor that is not a namespace with
        /// `EINVAL` before it looks at the caller's capabilities, so this
        /// runs the real system call and gets the same answer with or
        /// without privileges. The error names the namespace that failed.
        #[test]
        fn joining_a_descriptor_that_is_no_namespace_fails_with_einval() {
            let root = fake_proc(&["pid"], &["net", "uts"]);
            let handles = open_foreign_namespaces(root.path(), TARGET_PID);
            assert_eq!(names(&handles), ["net", "uts"]);

            assert_eq!(join_namespaces(&handles), Err(("net", Errno::EINVAL)));
        }
    }
}

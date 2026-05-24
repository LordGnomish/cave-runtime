// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: descriptor schema adapted from google/gvisor runsc/fsgofer/* (Apache-2.0).
//! gVisor Gofer — 9P file server proxy (runsc/fsgofer/).
//!
//! Gofer is a host-side process that proxies the Sentry's file ops over a
//! 9P-on-unix-socket transport. We model the *descriptor* (mount mapping)
//! and the file-descriptor handles. The real 9P server is OUT OF SCOPE.

use crate::oci_runtime_spec::Mount;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Gofer mount descriptor — passed to the gofer process at startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GoferMount {
    pub source: String,
    pub destination: String,
    pub readonly: bool,
    /// 9P mount tag (`gofer-rootfs-0`).
    pub tag: String,
}

impl GoferMount {
    pub fn from_oci(idx: usize, m: &Mount) -> Self {
        let readonly = m.options.iter().any(|o| o == "ro");
        GoferMount {
            source: m.source.clone(),
            destination: m.destination.clone(),
            readonly,
            tag: format!("gofer-{idx}"),
        }
    }
}

/// Gofer descriptor — full set of mounts + the rootfs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GoferDescriptor {
    pub rootfs: GoferMount,
    pub additional: Vec<GoferMount>,
}

impl GoferDescriptor {
    pub fn new(rootfs_src: impl Into<String>, readonly: bool) -> Self {
        GoferDescriptor {
            rootfs: GoferMount {
                source: rootfs_src.into(),
                destination: "/".into(),
                readonly,
                tag: "gofer-rootfs-0".into(),
            },
            additional: Vec::new(),
        }
    }

    /// All mounts — rootfs first, then `additional`.
    pub fn all_mounts(&self) -> Vec<&GoferMount> {
        let mut v = vec![&self.rootfs];
        v.extend(self.additional.iter());
        v
    }

    /// Number of FDs the gofer process needs (one per mount + control socket).
    pub fn fd_count(&self) -> usize {
        1 + self.all_mounts().len()
    }
}

/// FD table — `runsc/fsgofer/fsgofer.go::attachPoint`. Models the host-side
/// FD passed to the sentry over the control socket.
#[derive(Debug, Clone, Default)]
pub struct FdTable {
    inner: BTreeMap<i32, String>,
    next: i32,
}

impl FdTable {
    pub fn new() -> Self {
        FdTable { inner: BTreeMap::new(), next: 3 }
    }

    /// Insert a new mount-mapped FD. Returns the allocated FD number.
    pub fn insert(&mut self, path: impl Into<String>) -> i32 {
        let fd = self.next;
        self.next += 1;
        self.inner.insert(fd, path.into());
        fd
    }

    pub fn get(&self, fd: i32) -> Option<&str> {
        self.inner.get(&fd).map(|s| s.as_str())
    }

    pub fn remove(&mut self, fd: i32) -> Option<String> {
        self.inner.remove(&fd)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gofer_descriptor_starts_with_rootfs_only() {
        let g = GoferDescriptor::new("/rootfs", true);
        assert_eq!(g.all_mounts().len(), 1);
        assert!(g.rootfs.readonly);
    }

    #[test]
    fn fd_count_includes_control() {
        let mut g = GoferDescriptor::new("/rootfs", true);
        g.additional.push(GoferMount {
            source: "/proc".into(),
            destination: "/proc".into(),
            readonly: false,
            tag: "gofer-1".into(),
        });
        // 1 control + 1 rootfs + 1 extra = 3
        assert_eq!(g.fd_count(), 3);
    }

    #[test]
    fn from_oci_detects_readonly() {
        let m = Mount {
            destination: "/etc".into(),
            source: "/host/etc".into(),
            fs_type: "bind".into(),
            options: vec!["ro".into(), "nodev".into()],
        };
        let g = GoferMount::from_oci(2, &m);
        assert!(g.readonly);
        assert_eq!(g.tag, "gofer-2");
    }

    #[test]
    fn fdtable_starts_at_three() {
        let mut t = FdTable::new();
        let fd = t.insert("/x");
        assert_eq!(fd, 3);
        let fd2 = t.insert("/y");
        assert_eq!(fd2, 4);
    }

    #[test]
    fn fdtable_remove() {
        let mut t = FdTable::new();
        let fd = t.insert("/x");
        assert_eq!(t.get(fd), Some("/x"));
        assert_eq!(t.remove(fd).as_deref(), Some("/x"));
        assert_eq!(t.get(fd), None);
        assert!(t.is_empty());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Process model — credentials, namespaces, capabilities, and process tree.
//!
//! Upstream: `pkg/process/process.go`, `pkg/reader/exec/proc.go`,
//! `pkg/reader/caps/caps.go`, `pkg/reader/namespace/ns.go`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Process credentials — uid/gid sets + capability bitmasks +
/// secureBits + Linux namespaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub suid: u32,
    pub sgid: u32,
    pub fsuid: u32,
    pub fsgid: u32,
    pub caps_permitted: u64,
    pub caps_effective: u64,
    pub caps_inheritable: u64,
    pub caps_bset: u64,
    pub caps_ambient: u64,
    pub secure_bits: u32,
    pub user_ns: u64,
}

/// Linux namespace inode IDs (one per namespace type).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Namespaces {
    pub pid: u64,
    pub net: u64,
    pub mnt: u64,
    pub uts: u64,
    pub ipc: u64,
    pub cgroup: u64,
    pub user: u64,
    pub time: u64,
}

/// A single process, as Tetragon would emit it. Pids are namespace-aware:
/// `pid` is the host pid and `pid_in_ns` is the pid as seen inside the
/// container's pid namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Process {
    pub exec_id: String,
    pub pid: u32,
    pub pid_in_ns: u32,
    pub binary: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub cwd: String,
    pub credentials: Credentials,
    pub namespaces: Namespaces,
    pub parent_exec_id: Option<String>,
    #[serde(default)]
    pub container_id: Option<String>,
    #[serde(default)]
    pub pod_name: Option<String>,
    #[serde(default)]
    pub pod_namespace: Option<String>,
    pub start_time: DateTime<Utc>,
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,
}

impl Process {
    /// True if the process holds any of the capabilities passed in
    /// `caps_mask` in its effective set.
    pub fn has_effective_cap(&self, caps_mask: u64) -> bool {
        self.credentials.caps_effective & caps_mask != 0
    }

    /// True if the process is still alive (no end_time set).
    pub fn is_alive(&self) -> bool {
        self.end_time.is_none()
    }

    /// True if the process runs inside a container.
    pub fn is_containerized(&self) -> bool {
        self.container_id.is_some()
    }
}

/// In-memory process tree keyed by `exec_id`. Tetragon needs to walk
/// ancestry to attribute events; we store an O(1) lookup map.
#[derive(Debug, Default)]
pub struct ProcessTree {
    by_exec_id: BTreeMap<String, Process>,
    children: BTreeMap<String, Vec<String>>,
}

impl ProcessTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, p: Process) {
        if let Some(parent) = &p.parent_exec_id {
            self.children
                .entry(parent.clone())
                .or_default()
                .push(p.exec_id.clone());
        }
        self.by_exec_id.insert(p.exec_id.clone(), p);
    }

    pub fn get(&self, exec_id: &str) -> Option<&Process> {
        self.by_exec_id.get(exec_id)
    }

    pub fn children_of(&self, exec_id: &str) -> &[String] {
        self.children
            .get(exec_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Walk up the parent chain, returning all ancestor `exec_id`s in
    /// order from immediate parent up to the topmost known.
    pub fn ancestors(&self, exec_id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = self.by_exec_id.get(exec_id).and_then(|p| p.parent_exec_id.clone());
        while let Some(id) = cur {
            cur = self.by_exec_id.get(&id).and_then(|p| p.parent_exec_id.clone());
            out.push(id);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.by_exec_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_exec_id.is_empty()
    }

    /// Reap a process by `exec_id` — sets `end_time` rather than deleting,
    /// so chain-of-custody can still attribute later events.
    pub fn reap(&mut self, exec_id: &str, at: DateTime<Utc>) -> bool {
        if let Some(p) = self.by_exec_id.get_mut(exec_id) {
            p.end_time = Some(at);
            true
        } else {
            false
        }
    }

    pub fn live_count(&self) -> usize {
        self.by_exec_id.values().filter(|p| p.is_alive()).count()
    }
}

/// Capability bit constants — mirrors `include/uapi/linux/capability.h`.
pub mod cap {
    pub const CAP_CHOWN: u64 = 1 << 0;
    pub const CAP_DAC_OVERRIDE: u64 = 1 << 1;
    pub const CAP_DAC_READ_SEARCH: u64 = 1 << 2;
    pub const CAP_FOWNER: u64 = 1 << 3;
    pub const CAP_KILL: u64 = 1 << 5;
    pub const CAP_SETGID: u64 = 1 << 6;
    pub const CAP_SETUID: u64 = 1 << 7;
    pub const CAP_NET_BIND_SERVICE: u64 = 1 << 10;
    pub const CAP_NET_ADMIN: u64 = 1 << 12;
    pub const CAP_NET_RAW: u64 = 1 << 13;
    pub const CAP_SYS_MODULE: u64 = 1 << 16;
    pub const CAP_SYS_ADMIN: u64 = 1 << 21;
    pub const CAP_SYS_BOOT: u64 = 1 << 22;
    pub const CAP_SYS_PTRACE: u64 = 1 << 19;
    pub const CAP_BPF: u64 = 1 << 39;
    pub const CAP_PERFMON: u64 = 1 << 38;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(s: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(s, 0).unwrap()
    }

    fn proc(id: &str, parent: Option<&str>) -> Process {
        Process {
            exec_id: id.into(),
            pid: 100,
            pid_in_ns: 1,
            binary: "/bin/sh".into(),
            arguments: "-c true".into(),
            cwd: "/".into(),
            credentials: Credentials::default(),
            namespaces: Namespaces::default(),
            parent_exec_id: parent.map(String::from),
            container_id: None,
            pod_name: None,
            pod_namespace: None,
            start_time: t(1_700_000_000),
            end_time: None,
        }
    }

    #[test]
    fn test_process_alive_and_containerized() {
        let mut p = proc("a", None);
        assert!(p.is_alive());
        assert!(!p.is_containerized());
        p.container_id = Some("docker://abc".into());
        assert!(p.is_containerized());
        p.end_time = Some(t(1_700_000_100));
        assert!(!p.is_alive());
    }

    #[test]
    fn test_effective_capability_check() {
        let mut p = proc("a", None);
        p.credentials.caps_effective = cap::CAP_NET_ADMIN | cap::CAP_NET_RAW;
        assert!(p.has_effective_cap(cap::CAP_NET_ADMIN));
        assert!(!p.has_effective_cap(cap::CAP_SYS_ADMIN));
        assert!(p.has_effective_cap(cap::CAP_NET_ADMIN | cap::CAP_SYS_ADMIN));
    }

    #[test]
    fn test_tree_insert_and_get() {
        let mut t = ProcessTree::new();
        assert!(t.is_empty());
        t.insert(proc("a", None));
        assert_eq!(t.len(), 1);
        assert_eq!(t.get("a").unwrap().exec_id, "a");
    }

    #[test]
    fn test_tree_children_and_ancestors() {
        let mut t = ProcessTree::new();
        t.insert(proc("root", None));
        t.insert(proc("child", Some("root")));
        t.insert(proc("grandchild", Some("child")));
        assert_eq!(t.children_of("root"), &["child".to_string()]);
        let anc = t.ancestors("grandchild");
        assert_eq!(anc, vec!["child".to_string(), "root".to_string()]);
    }

    #[test]
    fn test_tree_reap_sets_end_time() {
        let mut tree = ProcessTree::new();
        tree.insert(proc("a", None));
        assert_eq!(tree.live_count(), 1);
        assert!(tree.reap("a", t(1_700_000_200)));
        assert_eq!(tree.live_count(), 0);
        assert!(tree.get("a").unwrap().end_time.is_some());
        assert!(!tree.reap("missing", t(0)));
    }

    #[test]
    fn test_credentials_default_zero() {
        let c = Credentials::default();
        assert_eq!(c.uid, 0);
        assert_eq!(c.caps_effective, 0);
    }

    #[test]
    fn test_namespaces_default_zero() {
        let ns = Namespaces::default();
        assert_eq!(ns.pid + ns.net + ns.mnt + ns.uts + ns.ipc + ns.cgroup, 0);
    }

    #[test]
    fn test_cap_constants_distinct() {
        let set = vec![
            cap::CAP_CHOWN,
            cap::CAP_DAC_OVERRIDE,
            cap::CAP_SETUID,
            cap::CAP_NET_ADMIN,
            cap::CAP_SYS_ADMIN,
            cap::CAP_BPF,
        ];
        let dedup: std::collections::BTreeSet<_> = set.iter().collect();
        assert_eq!(dedup.len(), set.len());
    }

    #[test]
    fn test_process_serde_roundtrip() {
        let p = proc("a", None);
        let j = serde_json::to_string(&p).unwrap();
        let back: Process = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn test_namespace_set_serde() {
        let ns = Namespaces {
            pid: 4026531836,
            net: 4026531840,
            mnt: 4026531841,
            uts: 4026531842,
            ipc: 4026531843,
            cgroup: 4026531844,
            user: 4026531845,
            time: 4026531846,
        };
        let j = serde_json::to_string(&ns).unwrap();
        let back: Namespaces = serde_json::from_str(&j).unwrap();
        assert_eq!(back, ns);
    }
}

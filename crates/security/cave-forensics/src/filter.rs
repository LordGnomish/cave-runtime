// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tetragon-style policy match filters.
//!
//! Upstream: `pkg/selectors/kernel_selectors.go`,
//! `pkg/k8s/apis/cilium.io/v1alpha1/types.go::KProbeSelector*`.
//!
//! Each `FilterGroup` is an AND across all of its `match*` sub-filters.
//! Multiple `FilterGroup`s on a single kprobe are OR'd.

use crate::error::{ForensicsError, Result};
use crate::events::KernelEvent;
use crate::process::Process;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FilterGroup {
    #[serde(default)]
    pub match_pids: Vec<MatchPid>,
    #[serde(default)]
    pub match_namespaces: Vec<MatchNamespace>,
    #[serde(default)]
    pub match_capabilities: Vec<MatchCapability>,
    #[serde(default)]
    pub match_binaries: Vec<MatchBinary>,
    #[serde(default)]
    pub match_args: Vec<MatchArg>,
    #[serde(default)]
    pub match_actions: Vec<MatchAction>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum FilterOp {
    In,
    NotIn,
    Equal,
    NotEqual,
    Prefix,
    Postfix,
    Mask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchPid {
    pub operator: FilterOp,
    #[serde(default)]
    pub is_namespace_pid: bool,
    #[serde(default)]
    pub follow_forks: bool,
    pub values: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchNamespace {
    pub namespace: NamespaceKind,
    pub operator: FilterOp,
    pub values: Vec<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum NamespaceKind {
    Pid,
    Net,
    Mnt,
    Uts,
    Ipc,
    Cgroup,
    User,
    Time,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchCapability {
    #[serde(default)]
    pub set: CapabilitySet,
    pub operator: FilterOp,
    pub values: Vec<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum CapabilitySet {
    #[default]
    Effective,
    Permitted,
    Inheritable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchBinary {
    pub operator: FilterOp,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchArg {
    pub index: u32,
    pub operator: FilterOp,
    pub values: Vec<String>,
}

/// `matchActions` controls policy enforcement decisions. Encoded as
/// PascalCase to match the upstream YAML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchAction {
    pub action: ActionKind,
    #[serde(default)]
    pub arg_error: Option<i32>,
    #[serde(default)]
    pub arg_sig: Option<i32>,
    #[serde(default)]
    pub arg_fd: Option<u32>,
    #[serde(default)]
    pub arg_name: Option<u32>,
    #[serde(default)]
    pub rate_limit: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ActionKind {
    Post,
    Override,
    Sigkill,
    FollowFd,
    Signal,
    NoPost,
    UnfollowFd,
    GetUrl,
    DnsLookup,
    NotifyEnforcer,
}

impl FilterGroup {
    /// Evaluate this filter group against a kernel event. Returns
    /// `Ok(true)` if the event matches every populated sub-filter.
    pub fn matches(&self, ev: &KernelEvent) -> Result<bool> {
        let Some(p) = ev.process() else {
            // Without a process the filter is structurally indeterminate;
            // treat as non-match (matches upstream behaviour).
            return Ok(self.is_empty());
        };
        for mp in &self.match_pids {
            if !mp.matches(p)? {
                return Ok(false);
            }
        }
        for mn in &self.match_namespaces {
            if !mn.matches(p) {
                return Ok(false);
            }
        }
        for mc in &self.match_capabilities {
            if !mc.matches(p) {
                return Ok(false);
            }
        }
        for mb in &self.match_binaries {
            if !mb.matches(p)? {
                return Ok(false);
            }
        }
        for ma in &self.match_args {
            if !ma.matches(ev)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn is_empty(&self) -> bool {
        self.match_pids.is_empty()
            && self.match_namespaces.is_empty()
            && self.match_capabilities.is_empty()
            && self.match_binaries.is_empty()
            && self.match_args.is_empty()
            && self.match_actions.is_empty()
    }

    /// All actions to enforce when this group matches.
    pub fn actions(&self) -> &[MatchAction] {
        &self.match_actions
    }
}

impl MatchPid {
    pub fn matches(&self, p: &Process) -> Result<bool> {
        let pid = if self.is_namespace_pid {
            p.pid_in_ns
        } else {
            p.pid
        };
        match self.operator {
            FilterOp::In | FilterOp::Equal => Ok(self.values.contains(&pid)),
            FilterOp::NotIn | FilterOp::NotEqual => Ok(!self.values.contains(&pid)),
            other => Err(ForensicsError::FilterOpRejected(
                format!("{other:?}"),
                "pid".into(),
            )),
        }
    }
}

impl MatchNamespace {
    pub fn matches(&self, p: &Process) -> bool {
        let ns = match self.namespace {
            NamespaceKind::Pid => p.namespaces.pid,
            NamespaceKind::Net => p.namespaces.net,
            NamespaceKind::Mnt => p.namespaces.mnt,
            NamespaceKind::Uts => p.namespaces.uts,
            NamespaceKind::Ipc => p.namespaces.ipc,
            NamespaceKind::Cgroup => p.namespaces.cgroup,
            NamespaceKind::User => p.namespaces.user,
            NamespaceKind::Time => p.namespaces.time,
        };
        match self.operator {
            FilterOp::In | FilterOp::Equal => self.values.contains(&ns),
            FilterOp::NotIn | FilterOp::NotEqual => !self.values.contains(&ns),
            _ => false,
        }
    }
}

impl MatchCapability {
    pub fn matches(&self, p: &Process) -> bool {
        let bits = match self.set {
            CapabilitySet::Effective => p.credentials.caps_effective,
            CapabilitySet::Permitted => p.credentials.caps_permitted,
            CapabilitySet::Inheritable => p.credentials.caps_inheritable,
        };
        match self.operator {
            FilterOp::Mask => self.values.iter().all(|v| bits & v == *v),
            FilterOp::In => self.values.iter().any(|v| bits & v != 0),
            FilterOp::NotIn => self.values.iter().all(|v| bits & v == 0),
            FilterOp::Equal => self.values.iter().any(|v| bits == *v),
            FilterOp::NotEqual => self.values.iter().all(|v| bits != *v),
            _ => false,
        }
    }
}

impl MatchBinary {
    pub fn matches(&self, p: &Process) -> Result<bool> {
        let bin = p.binary.as_str();
        match self.operator {
            FilterOp::In | FilterOp::Equal => Ok(self.values.iter().any(|v| v == bin)),
            FilterOp::NotIn | FilterOp::NotEqual => Ok(self.values.iter().all(|v| v != bin)),
            FilterOp::Prefix => Ok(self.values.iter().any(|v| bin.starts_with(v))),
            FilterOp::Postfix => Ok(self.values.iter().any(|v| bin.ends_with(v))),
            other => Err(ForensicsError::FilterOpRejected(
                format!("{other:?}"),
                "binary".into(),
            )),
        }
    }
}

impl MatchArg {
    pub fn matches(&self, ev: &KernelEvent) -> Result<bool> {
        let candidates = collect_arg_strings(ev, self.index);
        if candidates.is_empty() {
            return Ok(matches!(self.operator, FilterOp::NotEqual | FilterOp::NotIn));
        }
        let any = |pred: &dyn Fn(&str) -> bool| candidates.iter().any(|s| pred(s));
        let all = |pred: &dyn Fn(&str) -> bool| candidates.iter().all(|s| pred(s));
        match self.operator {
            FilterOp::In | FilterOp::Equal => Ok(any(&|s| self.values.iter().any(|v| v == s))),
            FilterOp::NotIn | FilterOp::NotEqual => {
                Ok(all(&|s| self.values.iter().all(|v| v != s)))
            }
            FilterOp::Prefix => Ok(any(&|s| self.values.iter().any(|v| s.starts_with(v)))),
            FilterOp::Postfix => Ok(any(&|s| self.values.iter().any(|v| s.ends_with(v)))),
            other => Err(ForensicsError::FilterOpRejected(
                format!("{other:?}"),
                "arg".into(),
            )),
        }
    }
}

/// Collect a kprobe/uprobe arg as a string for matching. Falls back to
/// well-known event-specific fields (path for FileEvent, dst_ip for
/// NetworkEvent, etc.).
fn collect_arg_strings(ev: &KernelEvent, index: u32) -> Vec<String> {
    match ev {
        KernelEvent::Kprobe(k) => k
            .args
            .iter()
            .filter(|a| a.index == index)
            .filter_map(|a| a.value.as_str().map(|s| s.to_string()))
            .collect(),
        KernelEvent::Uprobe(u) => u
            .args
            .iter()
            .filter(|a| a.index == index)
            .filter_map(|a| a.value.as_str().map(|s| s.to_string()))
            .collect(),
        KernelEvent::FileOp(f) if index == 0 => vec![f.path.clone()],
        KernelEvent::Network(n) if index == 0 => vec![format!("{}:{}", n.dst_ip, n.dst_port)],
        KernelEvent::ProcessExec(p) if index == 0 => vec![p.process.binary.clone()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::file::{FileEvent, FileOp};
    use crate::events::network::{ipv4, L4Proto, NetworkEvent, NetworkOp};
    use crate::events::process_exec::ProcessExecEvent;
    use crate::process::{cap, Credentials, Namespaces};
    use chrono::{TimeZone, Utc};

    fn ts() -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    fn proc(pid: u32, binary: &str) -> Process {
        let mut p = Process {
            exec_id: format!("e-{pid}"),
            pid,
            pid_in_ns: pid + 1,
            binary: binary.into(),
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
        };
        p.credentials.caps_effective = cap::CAP_NET_ADMIN | cap::CAP_NET_RAW;
        p.namespaces.pid = 4026531836;
        p
    }

    fn exec_ev(p: Process) -> KernelEvent {
        KernelEvent::ProcessExec(ProcessExecEvent {
            process: p,
            ancestors: vec![],
            observed_at: ts(),
        })
    }

    #[test]
    fn test_match_pid_in_operator() {
        let mp = MatchPid {
            operator: FilterOp::In,
            is_namespace_pid: false,
            follow_forks: false,
            values: vec![100, 200],
        };
        assert!(mp.matches(&proc(100, "/bin/sh")).unwrap());
        assert!(!mp.matches(&proc(101, "/bin/sh")).unwrap());
    }

    #[test]
    fn test_match_pid_namespace_pid_path() {
        let mp = MatchPid {
            operator: FilterOp::Equal,
            is_namespace_pid: true,
            follow_forks: false,
            values: vec![101],
        };
        // pid_in_ns = pid + 1
        assert!(mp.matches(&proc(100, "/bin/sh")).unwrap());
    }

    #[test]
    fn test_match_pid_rejects_prefix_op() {
        let mp = MatchPid {
            operator: FilterOp::Prefix,
            is_namespace_pid: false,
            follow_forks: false,
            values: vec![100],
        };
        let err = mp.matches(&proc(100, "/bin/sh")).unwrap_err();
        assert!(format!("{err}").contains("pid"));
    }

    #[test]
    fn test_match_namespace_in() {
        let mn = MatchNamespace {
            namespace: NamespaceKind::Pid,
            operator: FilterOp::In,
            values: vec![4026531836],
        };
        assert!(mn.matches(&proc(1, "/bin/sh")));
    }

    #[test]
    fn test_match_capability_mask_op() {
        let mc = MatchCapability {
            set: CapabilitySet::Effective,
            operator: FilterOp::Mask,
            values: vec![cap::CAP_NET_ADMIN],
        };
        assert!(mc.matches(&proc(1, "/bin/sh")));

        let mc = MatchCapability {
            set: CapabilitySet::Effective,
            operator: FilterOp::Mask,
            values: vec![cap::CAP_SYS_ADMIN],
        };
        assert!(!mc.matches(&proc(1, "/bin/sh")));
    }

    #[test]
    fn test_match_capability_in_op() {
        let mc = MatchCapability {
            set: CapabilitySet::Effective,
            operator: FilterOp::In,
            values: vec![cap::CAP_NET_RAW, cap::CAP_SYS_ADMIN],
        };
        assert!(mc.matches(&proc(1, "/bin/sh")));
    }

    #[test]
    fn test_match_binary_prefix() {
        let mb = MatchBinary {
            operator: FilterOp::Prefix,
            values: vec!["/usr/bin/".into()],
        };
        assert!(mb.matches(&proc(1, "/usr/bin/curl")).unwrap());
        assert!(!mb.matches(&proc(1, "/bin/sh")).unwrap());
    }

    #[test]
    fn test_match_binary_postfix() {
        let mb = MatchBinary {
            operator: FilterOp::Postfix,
            values: vec!["bash".into()],
        };
        assert!(mb.matches(&proc(1, "/bin/bash")).unwrap());
    }

    #[test]
    fn test_match_arg_against_file_event() {
        let p = proc(1, "/bin/cat");
        let ev = KernelEvent::FileOp(FileEvent {
            op: FileOp::Read,
            path: "/etc/shadow".into(),
            flags: None,
            mode: None,
            process: p,
            observed_at: ts(),
        });
        let ma = MatchArg {
            index: 0,
            operator: FilterOp::Equal,
            values: vec!["/etc/shadow".into()],
        };
        assert!(ma.matches(&ev).unwrap());
    }

    #[test]
    fn test_match_arg_against_network_event() {
        let p = proc(1, "/bin/curl");
        let ev = KernelEvent::Network(NetworkEvent {
            op: NetworkOp::Connect,
            proto: L4Proto::Tcp,
            src_ip: ipv4(10, 0, 0, 1),
            src_port: 33333,
            dst_ip: ipv4(1, 1, 1, 1),
            dst_port: 443,
            bytes: 0,
            process: p,
            observed_at: ts(),
        });
        let ma = MatchArg {
            index: 0,
            operator: FilterOp::Equal,
            values: vec!["1.1.1.1:443".into()],
        };
        assert!(ma.matches(&ev).unwrap());
    }

    #[test]
    fn test_filter_group_all_match() {
        let p = proc(100, "/bin/bash");
        let mut g = FilterGroup::default();
        g.match_pids.push(MatchPid {
            operator: FilterOp::In,
            is_namespace_pid: false,
            follow_forks: false,
            values: vec![100],
        });
        g.match_binaries.push(MatchBinary {
            operator: FilterOp::Postfix,
            values: vec!["bash".into()],
        });
        assert!(g.matches(&exec_ev(p)).unwrap());
    }

    #[test]
    fn test_filter_group_empty_is_match_all() {
        let g = FilterGroup::default();
        assert!(g.is_empty());
        let p = proc(1, "/bin/sh");
        assert!(g.matches(&exec_ev(p)).unwrap());
    }

    #[test]
    fn test_match_action_serde_roundtrip() {
        let a = MatchAction {
            action: ActionKind::Sigkill,
            arg_error: None,
            arg_sig: Some(9),
            arg_fd: None,
            arg_name: None,
            rate_limit: Some("5/m".into()),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: MatchAction = serde_json::from_str(&j).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn test_action_kind_pascal_case() {
        let j = serde_json::to_string(&ActionKind::FollowFd).unwrap();
        assert_eq!(j, "\"FollowFd\"");
    }

    #[test]
    fn test_match_namespace_serde_roundtrip() {
        let mn = MatchNamespace {
            namespace: NamespaceKind::Net,
            operator: FilterOp::NotIn,
            values: vec![123],
        };
        let j = serde_json::to_string(&mn).unwrap();
        let back: MatchNamespace = serde_json::from_str(&j).unwrap();
        assert_eq!(back, mn);
    }

    #[test]
    fn test_filter_group_short_circuits_on_first_mismatch() {
        let p = proc(100, "/bin/sh");
        let mut g = FilterGroup::default();
        g.match_pids.push(MatchPid {
            operator: FilterOp::In,
            is_namespace_pid: false,
            follow_forks: false,
            values: vec![999],
        });
        g.match_binaries.push(MatchBinary {
            operator: FilterOp::Prefix,
            values: vec!["/bin/".into()],
        });
        assert!(!g.matches(&exec_ev(p)).unwrap());
    }
}

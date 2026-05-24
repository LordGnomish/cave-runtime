// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tetragon enforcement actions (sigkill / override-return / follow-fd).
//!
//! Upstream: `pkg/sensors/tracing/genericKprobeSensor.go::doAction*`,
//! `pkg/enforcer/enforcer.go`.
//!
//! This module is a pure state-machine — no actual signals or ioctls are
//! emitted. cave-runtime hosts (cri-runtime + kubelet) consume the
//! `EnforcementDecision`s via the gRPC export stream and execute them
//! out-of-band. That keeps cave-forensics portable to non-Linux hosts.

use crate::error::Result;
use crate::events::KernelEvent;
use crate::filter::{ActionKind, FilterGroup, MatchAction};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Decision emitted by the enforcer for a single event/policy pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnforcementDecision {
    pub policy_name: String,
    pub event_kind: String,
    pub target_pid: u32,
    pub action: ActionKind,
    pub arg_error: Option<i32>,
    pub arg_sig: Option<i32>,
    pub arg_fd: Option<u32>,
}

/// In-memory follow-fd map — `pid -> Vec<fd>`. Mirrors the kernel-side
/// map that Tetragon populates from `matchActions[].FollowFD`.
#[derive(Debug, Default)]
pub struct FollowFdMap {
    inner: DashMap<u32, Vec<u32>>,
}

impl FollowFdMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn follow(&self, pid: u32, fd: u32) {
        self.inner.entry(pid).or_default().push(fd);
    }

    pub fn unfollow(&self, pid: u32, fd: u32) {
        if let Some(mut entry) = self.inner.get_mut(&pid) {
            entry.retain(|f| *f != fd);
        }
    }

    pub fn is_followed(&self, pid: u32, fd: u32) -> bool {
        self.inner
            .get(&pid)
            .map(|v| v.contains(&fd))
            .unwrap_or(false)
    }

    pub fn followed_fds(&self, pid: u32) -> Vec<u32> {
        self.inner.get(&pid).map(|v| v.clone()).unwrap_or_default()
    }

    pub fn pid_count(&self) -> usize {
        self.inner.len()
    }
}

/// Tetragon-equivalent enforcer engine. Holds the follow-fd map and a
/// monitor-mode override that suppresses all enforcement decisions.
pub struct Enforcer {
    monitor_only: bool,
    fds: Arc<FollowFdMap>,
}

impl Default for Enforcer {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Enforcer {
    pub fn new(monitor_only: bool) -> Self {
        Self {
            monitor_only,
            fds: Arc::new(FollowFdMap::new()),
        }
    }

    pub fn set_monitor_only(&mut self, on: bool) {
        self.monitor_only = on;
    }

    pub fn is_monitor_only(&self) -> bool {
        self.monitor_only
    }

    pub fn fds(&self) -> Arc<FollowFdMap> {
        Arc::clone(&self.fds)
    }

    /// Evaluate a single event against a filter group + decide on
    /// enforcement actions. Returns one decision per matchAction (empty
    /// if the group did not match or if in monitor mode).
    pub fn decide(
        &self,
        policy_name: &str,
        group: &FilterGroup,
        ev: &KernelEvent,
    ) -> Result<Vec<EnforcementDecision>> {
        if !group.matches(ev)? {
            return Ok(Vec::new());
        }
        if self.monitor_only {
            return Ok(Vec::new());
        }
        let target_pid = ev.process().map(|p| p.pid).unwrap_or(0);
        let kind_tag = ev.kind_tag().to_string();
        let mut out = Vec::with_capacity(group.actions().len());
        for a in group.actions() {
            self.apply_side_effects(target_pid, a);
            if matches!(a.action, ActionKind::Post | ActionKind::NoPost) {
                // Post/NoPost are observability hints, not enforcement.
                continue;
            }
            out.push(EnforcementDecision {
                policy_name: policy_name.to_string(),
                event_kind: kind_tag.clone(),
                target_pid,
                action: a.action,
                arg_error: a.arg_error,
                arg_sig: a.arg_sig,
                arg_fd: a.arg_fd,
            });
        }
        Ok(out)
    }

    fn apply_side_effects(&self, pid: u32, a: &MatchAction) {
        match a.action {
            ActionKind::FollowFd => {
                if let Some(fd) = a.arg_fd {
                    self.fds.follow(pid, fd);
                }
            }
            ActionKind::UnfollowFd => {
                if let Some(fd) = a.arg_fd {
                    self.fds.unfollow(pid, fd);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::process_exec::ProcessExecEvent;
    use crate::filter::{FilterOp, MatchAction, MatchBinary};
    use crate::process::{Credentials, Namespaces, Process};
    use chrono::{TimeZone, Utc};

    fn ts() -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    fn exec_event_for(binary: &str) -> KernelEvent {
        KernelEvent::ProcessExec(ProcessExecEvent {
            process: Process {
                exec_id: "x".into(),
                pid: 42,
                pid_in_ns: 1,
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
            },
            ancestors: vec![],
            observed_at: ts(),
        })
    }

    fn kill_group() -> FilterGroup {
        let mut g = FilterGroup::default();
        g.match_binaries.push(MatchBinary {
            operator: FilterOp::Equal,
            values: vec!["/bin/bash".into()],
        });
        g.match_actions.push(MatchAction {
            action: ActionKind::Sigkill,
            arg_error: None,
            arg_sig: Some(9),
            arg_fd: None,
            arg_name: None,
            rate_limit: None,
        });
        g
    }

    #[test]
    fn test_sigkill_decision_emitted_when_group_matches() {
        let e = Enforcer::default();
        let dec = e.decide("p1", &kill_group(), &exec_event_for("/bin/bash")).unwrap();
        assert_eq!(dec.len(), 1);
        assert_eq!(dec[0].action, ActionKind::Sigkill);
        assert_eq!(dec[0].arg_sig, Some(9));
        assert_eq!(dec[0].target_pid, 42);
        assert_eq!(dec[0].policy_name, "p1");
    }

    #[test]
    fn test_no_decision_when_group_does_not_match() {
        let e = Enforcer::default();
        let dec = e.decide("p1", &kill_group(), &exec_event_for("/bin/sh")).unwrap();
        assert!(dec.is_empty());
    }

    #[test]
    fn test_monitor_only_suppresses_decisions() {
        let mut e = Enforcer::default();
        e.set_monitor_only(true);
        assert!(e.is_monitor_only());
        let dec = e.decide("p1", &kill_group(), &exec_event_for("/bin/bash")).unwrap();
        assert!(dec.is_empty());
    }

    #[test]
    fn test_override_return_carries_errno() {
        let mut g = FilterGroup::default();
        g.match_binaries.push(MatchBinary {
            operator: FilterOp::Equal,
            values: vec!["/bin/cat".into()],
        });
        g.match_actions.push(MatchAction {
            action: ActionKind::Override,
            arg_error: Some(-1),
            arg_sig: None,
            arg_fd: None,
            arg_name: None,
            rate_limit: None,
        });
        let e = Enforcer::default();
        let dec = e.decide("override", &g, &exec_event_for("/bin/cat")).unwrap();
        assert_eq!(dec[0].action, ActionKind::Override);
        assert_eq!(dec[0].arg_error, Some(-1));
    }

    #[test]
    fn test_post_action_does_not_create_decision() {
        let mut g = FilterGroup::default();
        g.match_binaries.push(MatchBinary {
            operator: FilterOp::Equal,
            values: vec!["/bin/sh".into()],
        });
        g.match_actions.push(MatchAction {
            action: ActionKind::Post,
            arg_error: None,
            arg_sig: None,
            arg_fd: None,
            arg_name: None,
            rate_limit: None,
        });
        let e = Enforcer::default();
        let dec = e.decide("p", &g, &exec_event_for("/bin/sh")).unwrap();
        assert!(dec.is_empty(), "Post is observability-only");
    }

    #[test]
    fn test_follow_fd_map_follow_and_unfollow() {
        let m = FollowFdMap::new();
        m.follow(1, 7);
        m.follow(1, 8);
        m.follow(2, 9);
        assert!(m.is_followed(1, 7));
        assert!(m.is_followed(2, 9));
        m.unfollow(1, 7);
        assert!(!m.is_followed(1, 7));
        assert!(m.is_followed(1, 8));
        assert_eq!(m.pid_count(), 2);
    }

    #[test]
    fn test_follow_fd_action_writes_to_map() {
        let e = Enforcer::default();
        let mut g = FilterGroup::default();
        g.match_binaries.push(MatchBinary {
            operator: FilterOp::Equal,
            values: vec!["/bin/sh".into()],
        });
        g.match_actions.push(MatchAction {
            action: ActionKind::FollowFd,
            arg_error: None,
            arg_sig: None,
            arg_fd: Some(3),
            arg_name: Some(0),
            rate_limit: None,
        });
        let _ = e.decide("fd-p", &g, &exec_event_for("/bin/sh")).unwrap();
        assert!(e.fds().is_followed(42, 3));
    }

    #[test]
    fn test_followed_fds_for_unknown_pid_is_empty() {
        let m = FollowFdMap::new();
        assert!(m.followed_fds(999).is_empty());
    }

    #[test]
    fn test_enforcement_decision_serde_roundtrip() {
        let d = EnforcementDecision {
            policy_name: "p".into(),
            event_kind: "process_exec".into(),
            target_pid: 1,
            action: ActionKind::Sigkill,
            arg_error: None,
            arg_sig: Some(9),
            arg_fd: None,
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: EnforcementDecision = serde_json::from_str(&j).unwrap();
        assert_eq!(back, d);
    }
}

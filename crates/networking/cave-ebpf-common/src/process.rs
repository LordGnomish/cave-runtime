// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Process discovery + exec/exit watcher — userspace model of grafana/beyla
//! `pkg/internal/discover`.
//!
//! Beyla decides which processes to auto-instrument by matching each
//! process against discovery criteria (executable path and/or open TCP
//! ports), watches `/proc` for `exec` and `exit` events, and derives a
//! service name per process (an explicit override, else the executable
//! basename). This module ports that matching + watcher state machine; the
//! real `/proc` scan and the netlink exec/exit feed are supplied by the
//! caller (here, by the daemon) — the logic is identical.

use std::collections::HashMap;

/// A discovered process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub exe_path: String,
    pub comm: String,
    pub open_ports: Vec<u16>,
}

impl ProcessInfo {
    /// Derive the service name: the `override` if present, else the
    /// executable basename (Beyla's default `service.name`).
    pub fn service_name(&self, override_name: Option<&str>) -> String {
        if let Some(o) = override_name {
            return o.to_string();
        }
        self.exe_path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.exe_path)
            .to_string()
    }
}

/// An inclusive TCP port range, e.g. `8080-8089` or a single `9000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRange {
    pub lo: u16,
    pub hi: u16,
}

impl PortRange {
    /// Parse `"lo-hi"` or a single `"port"`. Returns `None` on malformed
    /// input or an inverted range.
    pub fn parse(s: &str) -> Option<PortRange> {
        let s = s.trim();
        if let Some((a, b)) = s.split_once('-') {
            let lo: u16 = a.trim().parse().ok()?;
            let hi: u16 = b.trim().parse().ok()?;
            if lo > hi {
                return None;
            }
            Some(PortRange { lo, hi })
        } else {
            let p: u16 = s.parse().ok()?;
            Some(PortRange { lo: p, hi: p })
        }
    }

    pub fn contains(&self, port: u16) -> bool {
        self.lo <= port && port <= self.hi
    }
}

/// Discovery selector. Selectors present are ANDed; at least one must be
/// set for the criteria to match anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Criteria {
    /// Match when the process `exe_path` contains this substring.
    pub exe_substring: Option<String>,
    /// Match when any of the process's open ports falls in this range.
    pub ports: Option<PortRange>,
}

impl Criteria {
    pub fn matches(&self, info: &ProcessInfo) -> bool {
        // Empty criteria match nothing — Beyla requires explicit discovery.
        if self.exe_substring.is_none() && self.ports.is_none() {
            return false;
        }
        if let Some(sub) = &self.exe_substring {
            if !info.exe_path.contains(sub.as_str()) {
                return false;
            }
        }
        if let Some(range) = &self.ports {
            if !info.open_ports.iter().any(|p| range.contains(*p)) {
                return false;
            }
        }
        true
    }
}

/// Stateful watcher: tracks processes that match any configured criteria.
#[derive(Debug, Clone, Default)]
pub struct ProcessWatcher {
    criteria: Vec<Criteria>,
    tracked: HashMap<u32, ProcessInfo>,
}

impl ProcessWatcher {
    pub fn new(criteria: Vec<Criteria>) -> Self {
        Self {
            criteria,
            tracked: HashMap::new(),
        }
    }

    fn matches_any(&self, info: &ProcessInfo) -> bool {
        self.criteria.iter().any(|c| c.matches(info))
    }

    /// Handle an `exec`: if the process matches any criteria, start
    /// tracking it and return a reference; otherwise return `None`.
    pub fn on_exec(&mut self, info: ProcessInfo) -> Option<&ProcessInfo> {
        if self.matches_any(&info) {
            let pid = info.pid;
            self.tracked.insert(pid, info);
            self.tracked.get(&pid)
        } else {
            None
        }
    }

    /// Handle an `exit`: stop tracking. Returns whether the pid was tracked.
    pub fn on_exit(&mut self, pid: u32) -> bool {
        self.tracked.remove(&pid).is_some()
    }

    pub fn is_tracked(&self, pid: u32) -> bool {
        self.tracked.contains_key(&pid)
    }

    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }

    pub fn get(&self, pid: u32) -> Option<&ProcessInfo> {
        self.tracked.get(&pid)
    }
}

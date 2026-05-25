// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Falco event types — wire shape of events that the userspace engine
//! receives from either libsinsp (syscalls) or the k8s_audit plugin.
//!
//! NOTICE: upstream is falcosecurity/falco (Apache-2.0). The wire shape
//! mirrors `userspace/falco/falco_engine.h` (`sinsp_evt` field set) and
//! `plugins/k8saudit/src/k8saudit.cpp` (k8s_audit event source).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Priority {
    Emergency,
    Alert,
    Critical,
    Error,
    Warning,
    Notice,
    Informational,
    Debug,
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Emergency => "EMERGENCY",
            Priority::Alert => "ALERT",
            Priority::Critical => "CRITICAL",
            Priority::Error => "ERROR",
            Priority::Warning => "WARNING",
            Priority::Notice => "NOTICE",
            Priority::Informational => "INFORMATIONAL",
            Priority::Debug => "DEBUG",
        }
    }

    /// Compare severity (Emergency = max).
    pub fn rank(&self) -> u8 {
        match self {
            Priority::Emergency => 7,
            Priority::Alert => 6,
            Priority::Critical => 5,
            Priority::Error => 4,
            Priority::Warning => 3,
            Priority::Notice => 2,
            Priority::Informational => 1,
            Priority::Debug => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventClass {
    Syscall,
    K8sAudit,
    Plugin,
    Internal,
}

/// A single Falco event as exchanged with the rule engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FalcoEvent {
    pub class: EventClass,
    pub source: String,
    pub timestamp_ns: i64,
    /// Field bag — engine reads keys like `proc.name`, `evt.type`,
    /// `fd.name`, `ka.target.resource`. Always represented as strings;
    /// the engine knows how to coerce.
    pub fields: BTreeMap<String, String>,
}

impl FalcoEvent {
    pub fn syscall(name: impl Into<String>) -> Self {
        let mut e = FalcoEvent::default();
        e.class = EventClass::Syscall;
        e.source = "syscall".into();
        e.fields.insert("evt.type".into(), name.into());
        e
    }

    pub fn k8s_audit(stage: impl Into<String>, verb: impl Into<String>) -> Self {
        let mut e = FalcoEvent::default();
        e.class = EventClass::K8sAudit;
        e.source = "k8s_audit".into();
        e.fields.insert("ka.stage".into(), stage.into());
        e.fields.insert("ka.verb".into(), verb.into());
        e
    }

    pub fn with(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.fields.insert(k.into(), v.into());
        self
    }
}

impl Default for FalcoEvent {
    fn default() -> Self {
        Self {
            class: EventClass::Syscall,
            source: String::new(),
            timestamp_ns: 0,
            fields: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_rank_order_is_descending() {
        assert!(Priority::Emergency.rank() > Priority::Critical.rank());
        assert!(Priority::Warning.rank() > Priority::Informational.rank());
        assert_eq!(Priority::Debug.rank(), 0);
    }

    #[test]
    fn priority_str_matches_uppercase() {
        assert_eq!(Priority::Critical.as_str(), "CRITICAL");
        assert_eq!(Priority::Informational.as_str(), "INFORMATIONAL");
    }

    #[test]
    fn syscall_event_carries_evt_type() {
        let e = FalcoEvent::syscall("execve").with("proc.name", "bash");
        assert_eq!(e.class, EventClass::Syscall);
        assert_eq!(e.fields.get("evt.type").unwrap(), "execve");
        assert_eq!(e.fields.get("proc.name").unwrap(), "bash");
    }

    #[test]
    fn k8s_audit_event_carries_stage_and_verb() {
        let e = FalcoEvent::k8s_audit("ResponseComplete", "create");
        assert_eq!(e.source, "k8s_audit");
        assert_eq!(e.fields.get("ka.verb").unwrap(), "create");
    }

    #[test]
    fn event_serde_round_trips_via_json() {
        let e = FalcoEvent::syscall("openat").with("fd.name", "/etc/passwd");
        let j = serde_json::to_string(&e).unwrap();
        let r: FalcoEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(e, r);
    }

    #[test]
    fn priority_serde_uses_uppercase() {
        let j = serde_json::to_string(&Priority::Critical).unwrap();
        assert_eq!(j, "\"CRITICAL\"");
    }
}

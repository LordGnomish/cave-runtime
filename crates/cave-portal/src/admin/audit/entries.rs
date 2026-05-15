//! Audit entry types + in-memory ring buffer store.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    Read,
    Write,
    Delete,
    /// Pause / resume style mutation that flips an enable bit.
    Toggle,
    /// Out-of-band operator action (failover trigger, manual restart).
    Operate,
    /// A login / WebAuthn ceremony / role assumption.
    AuthEvent,
}

impl AuditAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            AuditAction::Read => "read",
            AuditAction::Write => "write",
            AuditAction::Delete => "delete",
            AuditAction::Toggle => "toggle",
            AuditAction::Operate => "operate",
            AuditAction::AuthEvent => "auth",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditResult {
    Ok,
    Denied,
    Error,
}

impl AuditResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            AuditResult::Ok => "ok",
            AuditResult::Denied => "denied",
            AuditResult::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp_unix: i64,
    pub persona: String,
    pub action: AuditAction,
    /// Resource the action targeted (`vault/secret/db-password`,
    /// `keda/scaledobject/echo`, ...).
    pub target: String,
    pub result: AuditResult,
    pub detail: String,
}

/// Bounded ring of audit entries. Oldest entries fall off when full.
#[derive(Debug)]
pub struct AuditStore {
    entries: RwLock<VecDeque<AuditEntry>>,
    capacity: usize,
}

impl AuditStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn append(&self, entry: AuditEntry) {
        let mut q = self.entries.write().unwrap();
        if q.len() == self.capacity {
            q.pop_front();
        }
        q.push_back(entry);
    }

    /// Snapshot of every entry, sorted newest-first.
    pub fn list(&self) -> Vec<AuditEntry> {
        let mut v: Vec<AuditEntry> = self.entries.read().unwrap().iter().cloned().collect();
        v.sort_by(|a, b| b.timestamp_unix.cmp(&a.timestamp_unix));
        v
    }

    /// Helper that records an action with `now_unix()` and the
    /// provided fields. Used by handlers that don't want to build
    /// `AuditEntry` by hand.
    pub fn record(
        &self,
        persona: impl Into<String>,
        action: AuditAction,
        target: impl Into<String>,
        result: AuditResult,
        detail: impl Into<String>,
    ) {
        self.append(AuditEntry {
            timestamp_unix: now_unix(),
            persona: persona.into(),
            action,
            target: target.into(),
            result,
            detail: detail.into(),
        });
    }
}

impl Default for AuditStore {
    fn default() -> Self {
        Self::new(10_000)
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_grows_until_capacity_then_evicts() {
        let s = AuditStore::new(3);
        for i in 0..5 {
            s.append(AuditEntry {
                timestamp_unix: i,
                persona: "p".into(),
                action: AuditAction::Read,
                target: "t".into(),
                result: AuditResult::Ok,
                detail: String::new(),
            });
        }
        assert_eq!(s.len(), 3);
        let entries = s.list();
        // Newest first → 4, 3, 2.
        assert_eq!(entries[0].timestamp_unix, 4);
        assert_eq!(entries[2].timestamp_unix, 2);
    }

    #[test]
    fn list_returns_newest_first() {
        let s = AuditStore::new(10);
        s.append(AuditEntry {
            timestamp_unix: 100,
            persona: "p".into(),
            action: AuditAction::Read,
            target: "t".into(),
            result: AuditResult::Ok,
            detail: String::new(),
        });
        s.append(AuditEntry {
            timestamp_unix: 50,
            persona: "p".into(),
            action: AuditAction::Read,
            target: "t".into(),
            result: AuditResult::Ok,
            detail: String::new(),
        });
        let l = s.list();
        assert_eq!(l[0].timestamp_unix, 100);
        assert_eq!(l[1].timestamp_unix, 50);
    }

    #[test]
    fn record_helper_stamps_timestamp() {
        let s = AuditStore::new(10);
        s.record("alice", AuditAction::Write, "x", AuditResult::Ok, "ok");
        assert_eq!(s.len(), 1);
        let e = &s.list()[0];
        assert_eq!(e.persona, "alice");
        assert!(e.timestamp_unix > 0);
    }

    #[test]
    fn action_strings_stable() {
        assert_eq!(AuditAction::Read.as_str(), "read");
        assert_eq!(AuditAction::Toggle.as_str(), "toggle");
        assert_eq!(AuditResult::Denied.as_str(), "denied");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Entry-event stream + agent fingerprint rebind primitives.
//!
//! Two upstream surfaces are absorbed here:
//!
//! - **registration-entry-event-stream** — SPIRE-server's
//!   `EntryClient.StreamEntries` gRPC bidi-stream pushes
//!   ADD/UPDATE/DELETE events for registration entries. Our HTTP
//!   surface is poll-only today; this module models the **event
//!   buffer** + **revision cursor** so agents poll
//!   `/api/identity/entries/events?since=<rev>` and receive
//!   deltas without re-fetching the world. A future SSE / WebSocket
//!   transport wraps this in cave-streams; the buffer abstraction
//!   stays the same.
//!
//! - **agent-fingerprint-rebinding** — When an attested agent renews
//!   its node-attestor certificate (new serial), SPIRE-server
//!   rebinds the parent of all entries the agent owned to the new
//!   serial. Without rebind the new SVID-issuer chain looks
//!   detached. We model a `Rebind` record + a state-machine that
//!   rejects rebinds across trust-domains and emits an entry-event
//!   per rebind so agents discover the new parent on the next poll.
//!
//! NOTICE: upstream is spiffe/spire (Apache-2.0). gRPC streaming +
//! the network transport live outside this crate — `EntryClient`
//! streaming itself is a cave-streams concern. The state machine,
//! buffer, and cursor are pure-Rust here.

use crate::error::{IdentityError, Result};
use crate::models::RegistrationEntry;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryEventKind {
    Add,
    Update,
    Delete,
    /// Emitted when an agent's parent-serial changes (rebind flow).
    ParentRebind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryEvent {
    pub revision: u64,
    pub kind: EntryEventKind,
    pub entry_id: String,
    pub spiffe_id: String,
    /// For `ParentRebind` only — previous parent SPIFFE ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_parent: Option<String>,
    /// For `ParentRebind` only — new parent SPIFFE ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_parent: Option<String>,
}

/// Ring-buffer of entry events keyed by monotonic revision.
///
/// Cursor: each poll returns events with `revision > since`. We cap
/// the buffer at `capacity` events; oldest are dropped (`Add` events
/// for stale entries can be re-derived from the full entry listing,
/// so dropping them is acceptable).
pub struct EntryEventBuffer {
    inner: Mutex<EventBufInner>,
    capacity: usize,
}

struct EventBufInner {
    next_rev: u64,
    events: VecDeque<EntryEvent>,
}

impl EntryEventBuffer {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(EventBufInner { next_rev: 1, events: VecDeque::with_capacity(capacity) }),
            capacity,
        })
    }

    fn push_event(&self, mut ev: EntryEvent) -> u64 {
        let mut g = self.inner.lock().unwrap();
        ev.revision = g.next_rev;
        g.next_rev += 1;
        if g.events.len() >= self.capacity {
            g.events.pop_front();
        }
        g.events.push_back(ev);
        g.next_rev - 1
    }

    pub fn record_add(&self, entry: &RegistrationEntry) -> u64 {
        self.push_event(EntryEvent {
            revision: 0,
            kind: EntryEventKind::Add,
            entry_id: entry.id.clone(),
            spiffe_id: entry.spiffe_id.to_string(),
            old_parent: None,
            new_parent: None,
        })
    }

    pub fn record_update(&self, entry: &RegistrationEntry) -> u64 {
        self.push_event(EntryEvent {
            revision: 0,
            kind: EntryEventKind::Update,
            entry_id: entry.id.clone(),
            spiffe_id: entry.spiffe_id.to_string(),
            old_parent: None,
            new_parent: None,
        })
    }

    pub fn record_delete(&self, entry_id: &str, spiffe_id: &str) -> u64 {
        self.push_event(EntryEvent {
            revision: 0,
            kind: EntryEventKind::Delete,
            entry_id: entry_id.into(),
            spiffe_id: spiffe_id.into(),
            old_parent: None,
            new_parent: None,
        })
    }

    pub fn record_rebind(&self, rebind: &AgentRebind) -> u64 {
        self.push_event(EntryEvent {
            revision: 0,
            kind: EntryEventKind::ParentRebind,
            entry_id: rebind.entry_id.clone(),
            spiffe_id: rebind.entry_spiffe_id.clone(),
            old_parent: Some(rebind.old_parent_id.clone()),
            new_parent: Some(rebind.new_parent_id.clone()),
        })
    }

    /// Read all events with `revision > since`. Returns the events +
    /// the next cursor (the highest revision returned, or `since`
    /// itself if no events).
    pub fn poll_since(&self, since: u64) -> (Vec<EntryEvent>, u64) {
        let g = self.inner.lock().unwrap();
        let mut out: Vec<EntryEvent> = g.events.iter().filter(|e| e.revision > since).cloned().collect();
        out.sort_by_key(|e| e.revision);
        let cursor = out.last().map(|e| e.revision).unwrap_or(since);
        (out, cursor)
    }

    pub fn len(&self) -> usize { self.inner.lock().unwrap().events.len() }
    pub fn is_empty(&self) -> bool { self.inner.lock().unwrap().events.is_empty() }
    pub fn next_revision(&self) -> u64 { self.inner.lock().unwrap().next_rev }
}

/// Agent fingerprint rebind record. Emitted when an agent renews its
/// node-attestor certificate (new x509 serial) and the server must
/// reparent the entries the old serial owned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRebind {
    pub entry_id: String,
    pub entry_spiffe_id: String,
    pub old_parent_id: String,
    pub new_parent_id: String,
    pub trust_domain: String,
}

impl AgentRebind {
    /// Validate: rebind MUST stay within the same trust domain;
    /// cross-TD rebind is a Charter v2 security violation (a
    /// compromised agent would re-parent entries to a foreign TD).
    pub fn validate(&self) -> Result<()> {
        let old_td = trust_domain_of(&self.old_parent_id);
        let new_td = trust_domain_of(&self.new_parent_id);
        let entry_td = trust_domain_of(&self.entry_spiffe_id);
        if old_td != new_td {
            return Err(IdentityError::Internal(format!(
                "rebind across trust-domains rejected: old='{}' new='{}'", old_td, new_td
            )));
        }
        if entry_td != new_td {
            return Err(IdentityError::Internal(format!(
                "rebind would orphan entry across trust-domain: entry='{}' parent='{}'", entry_td, new_td
            )));
        }
        if self.trust_domain != new_td {
            return Err(IdentityError::Internal(format!(
                "declared trust_domain='{}' doesn't match parent SPIFFE id '{}'",
                self.trust_domain, self.new_parent_id
            )));
        }
        Ok(())
    }
}

fn trust_domain_of(spiffe_id: &str) -> String {
    // SPIFFE ID is "spiffe://<trust-domain>/<path>"
    spiffe_id
        .strip_prefix("spiffe://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RegistrationEntry;

    fn entry(id: &str, spiffe: &str) -> RegistrationEntry {
        use crate::models::SpiffeId;
        let mut e = RegistrationEntry::default();
        e.id = id.into();
        e.spiffe_id = SpiffeId::new(spiffe);
        e.parent_id = SpiffeId::new("spiffe://example.org/spire/agent/k8s/node-A");
        e
    }

    fn rebind() -> AgentRebind {
        AgentRebind {
            entry_id: "e1".into(),
            entry_spiffe_id: "spiffe://example.org/ns/dev/sa/app".into(),
            old_parent_id: "spiffe://example.org/spire/agent/k8s/node-A".into(),
            new_parent_id: "spiffe://example.org/spire/agent/k8s/node-B".into(),
            trust_domain: "example.org".into(),
        }
    }

    #[test]
    fn new_buffer_is_empty_with_revision_one() {
        let b = EntryEventBuffer::new(16);
        assert!(b.is_empty());
        assert_eq!(b.next_revision(), 1);
    }

    #[test]
    fn add_assigns_monotonic_revisions() {
        let b = EntryEventBuffer::new(16);
        let r1 = b.record_add(&entry("e1", "spiffe://x/a"));
        let r2 = b.record_add(&entry("e2", "spiffe://x/b"));
        assert_eq!(r1, 1);
        assert_eq!(r2, 2);
    }

    #[test]
    fn poll_since_returns_only_newer_events() {
        let b = EntryEventBuffer::new(16);
        b.record_add(&entry("e1", "spiffe://x/a"));
        b.record_add(&entry("e2", "spiffe://x/b"));
        b.record_add(&entry("e3", "spiffe://x/c"));
        let (evs, cursor) = b.poll_since(1);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].entry_id, "e2");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn poll_since_at_head_returns_empty_and_same_cursor() {
        let b = EntryEventBuffer::new(16);
        b.record_add(&entry("e1", "spiffe://x/a"));
        let (evs, c) = b.poll_since(1);
        assert!(evs.is_empty());
        assert_eq!(c, 1);
    }

    #[test]
    fn capacity_drops_oldest() {
        let b = EntryEventBuffer::new(2);
        b.record_add(&entry("e1", "spiffe://x/a"));
        b.record_add(&entry("e2", "spiffe://x/b"));
        b.record_add(&entry("e3", "spiffe://x/c"));
        let (evs, _c) = b.poll_since(0);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].entry_id, "e2");
    }

    #[test]
    fn update_event_kind_recorded() {
        let b = EntryEventBuffer::new(8);
        b.record_update(&entry("e1", "spiffe://x/a"));
        let (evs, _) = b.poll_since(0);
        assert_eq!(evs[0].kind, EntryEventKind::Update);
    }

    #[test]
    fn delete_event_carries_id_and_spiffe() {
        let b = EntryEventBuffer::new(8);
        b.record_delete("e1", "spiffe://x/a");
        let (evs, _) = b.poll_since(0);
        assert_eq!(evs[0].kind, EntryEventKind::Delete);
        assert_eq!(evs[0].entry_id, "e1");
        assert_eq!(evs[0].spiffe_id, "spiffe://x/a");
    }

    #[test]
    fn rebind_event_carries_old_and_new_parent() {
        let b = EntryEventBuffer::new(8);
        b.record_rebind(&rebind());
        let (evs, _) = b.poll_since(0);
        assert_eq!(evs[0].kind, EntryEventKind::ParentRebind);
        assert!(evs[0].old_parent.as_ref().unwrap().contains("node-A"));
        assert!(evs[0].new_parent.as_ref().unwrap().contains("node-B"));
    }

    #[test]
    fn rebind_validate_same_td_succeeds() {
        rebind().validate().unwrap();
    }

    #[test]
    fn rebind_validate_cross_td_rejected() {
        let mut r = rebind();
        r.new_parent_id = "spiffe://other.org/spire/agent/x".into();
        r.trust_domain = "other.org".into(); // entry would orphan
        assert!(r.validate().is_err());
    }

    #[test]
    fn rebind_validate_entry_orphan_rejected() {
        let mut r = rebind();
        r.entry_spiffe_id = "spiffe://other.org/ns/dev/sa/app".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn rebind_validate_declared_td_must_match_parent() {
        let mut r = rebind();
        r.trust_domain = "wrong.org".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn trust_domain_extractor_returns_authority() {
        assert_eq!(trust_domain_of("spiffe://example.org/path/here"), "example.org");
        assert_eq!(trust_domain_of("not-a-spiffe-id"), "");
    }

    #[test]
    fn event_serde_json_round_trip() {
        let b = EntryEventBuffer::new(8);
        b.record_rebind(&rebind());
        let (evs, _) = b.poll_since(0);
        let j = serde_json::to_string(&evs[0]).unwrap();
        let r: EntryEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(r, evs[0]);
    }

    #[test]
    fn poll_returns_events_sorted_by_revision() {
        let b = EntryEventBuffer::new(8);
        b.record_add(&entry("e1", "spiffe://x/a"));
        b.record_update(&entry("e2", "spiffe://x/b"));
        b.record_delete("e3", "spiffe://x/c");
        let (evs, _) = b.poll_since(0);
        assert_eq!(evs.len(), 3);
        assert!(evs.windows(2).all(|w| w[0].revision < w[1].revision));
    }
}

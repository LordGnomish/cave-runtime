// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Audit event listener — `EventListenerProvider` parity. Every realm /
//! user / role / client / session mutation appends an `AuditEvent` to an
//! in-memory bounded sink; cave-logs consumes the drain.
//!
//! Upstream: `events/api/src/main/java/org/keycloak/events/EventListenerProvider.java`
//! + `services/src/main/java/org/keycloak/services/managers/RealmManager.java`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    // realm + user + role + client lifecycle
    RealmCreated,
    RealmDeleted,
    UserCreated,
    UserUpdated,
    UserDeleted,
    RoleCreated,
    RoleDeleted,
    ClientCreated,
    ClientDeleted,
    // auth lifecycle (subset of Keycloak `EventType`)
    Login,
    LoginError,
    Logout,
    TokenIssued,
    TokenRefreshed,
    TokenRevoked,
    PasswordChanged,
    OtpEnrolled,
    WebauthnEnrolled,
    AccountLocked,
    AccountUnlocked,
    BrokeredLogin,
    SamlLogin,
}

/// Audit event — Keycloak `Event.java` shape (subset).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub tenant_id: String,
    pub subject: String,
    pub kind: EventKind,
    pub at: DateTime<Utc>,
    pub client_id: Option<String>,
    pub ip_address: Option<String>,
    pub detail: Option<String>,
}

impl AuditEvent {
    pub fn new(tenant_id: impl Into<String>, subject: impl Into<String>, kind: EventKind) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            subject: subject.into(),
            kind,
            at: Utc::now(),
            client_id: None,
            ip_address: None,
            detail: None,
        }
    }

    pub fn with_client(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    pub fn with_ip(mut self, ip: impl Into<String>) -> Self {
        self.ip_address = Some(ip.into());
        self
    }

    pub fn with_detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}

/// In-memory bounded event sink (default cap 8192). When full the oldest
/// event is dropped (FIFO) so the listener never blocks the caller.
pub struct EventSink {
    inner: Mutex<EventSinkInner>,
}

struct EventSinkInner {
    events: Vec<AuditEvent>,
    cap: usize,
    dropped: u64,
}

impl Default for EventSink {
    fn default() -> Self {
        Self::with_capacity(8192)
    }
}

impl EventSink {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Mutex::new(EventSinkInner {
                events: Vec::new(),
                cap,
                dropped: 0,
            }),
        }
    }

    pub fn append(&self, e: AuditEvent) {
        let mut g = self.inner.lock().unwrap();
        if g.events.len() == g.cap {
            g.events.remove(0);
            g.dropped += 1;
        }
        g.events.push(e);
    }

    pub fn drain(&self) -> Vec<AuditEvent> {
        let mut g = self.inner.lock().unwrap();
        std::mem::take(&mut g.events)
    }

    pub fn snapshot(&self) -> Vec<AuditEvent> {
        let g = self.inner.lock().unwrap();
        g.events.clone()
    }

    pub fn dropped_count(&self) -> u64 {
        let g = self.inner.lock().unwrap();
        g.dropped
    }

    pub fn len(&self) -> usize {
        let g = self.inner.lock().unwrap();
        g.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_drain_roundtrip() {
        let s = EventSink::default();
        s.append(AuditEvent::new("t1", "u1", EventKind::Login));
        s.append(AuditEvent::new("t1", "u2", EventKind::Login));
        let drained = s.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].subject, "u1");
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn fifo_drop_when_capacity_exceeded() {
        let s = EventSink::with_capacity(2);
        s.append(AuditEvent::new("t1", "a", EventKind::Login));
        s.append(AuditEvent::new("t1", "b", EventKind::Login));
        s.append(AuditEvent::new("t1", "c", EventKind::Login));
        assert_eq!(s.dropped_count(), 1);
        let drained = s.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].subject, "b");
        assert_eq!(drained[1].subject, "c");
    }

    #[test]
    fn builder_attaches_metadata() {
        let e = AuditEvent::new("t1", "u1", EventKind::Login)
            .with_client("spa")
            .with_ip("10.0.0.1")
            .with_detail("first-login");
        assert_eq!(e.client_id.as_deref(), Some("spa"));
        assert_eq!(e.ip_address.as_deref(), Some("10.0.0.1"));
        assert_eq!(e.detail.as_deref(), Some("first-login"));
    }

    #[test]
    fn snapshot_does_not_drain() {
        let s = EventSink::default();
        s.append(AuditEvent::new("t1", "u1", EventKind::Login));
        let snap = s.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(s.len(), 1);
    }
}

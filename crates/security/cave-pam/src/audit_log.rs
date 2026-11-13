// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Structured audit log for privileged access management.
//!
//! Every security-relevant event (session open/close, access grant/deny,
//! command execution, policy changes) is appended here. Modelled after
//! Teleport's audit log API with queryable event stream.

use chrono::{DateTime, Utc};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ── Domain types ──────────────────────────────────────────────────────────────

/// Categories of audit-log events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEventKind {
    /// A privileged session was opened.
    SessionStart,
    /// A privileged session was closed.
    SessionEnd,
    /// A shell command was executed inside a session.
    CommandRun,
    /// An access request was granted (manually or automatically).
    AccessGranted,
    /// An access request was denied.
    AccessDenied,
    /// A user was enrolled or removed.
    UserChange,
    /// A role was created, modified, or deleted.
    RoleChange,
    /// A node was enrolled or removed from inventory.
    NodeChange,
    /// An authentication attempt was made.
    AuthAttempt,
    /// A policy was evaluated.
    PolicyEval,
}

/// An audit event submitted by the PAM plane.
pub struct AuditEvent {
    /// Event type.
    pub kind: AuditEventKind,
    /// User or service that triggered the event.
    pub actor_id: Uuid,
    /// The resource that was acted upon (hostname, DB name, etc.).
    pub target: String,
    /// Arbitrary structured context.
    pub metadata: serde_json::Value,
}

/// A stored audit record with a monotonic sequence ID and wall-clock time.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    /// Monotonic sequence number (unique within this log instance).
    pub seq: u64,
    /// When the event was recorded.
    pub recorded_at: DateTime<Utc>,
    /// Event category.
    pub kind: AuditEventKind,
    /// Principal that caused the event.
    pub actor_id: Uuid,
    /// Target resource name.
    pub target: String,
    /// Structured metadata.
    pub metadata: serde_json::Value,
}

// ── Query ─────────────────────────────────────────────────────────────────────

/// Parameters for querying the audit log.
#[derive(Debug, Default, Clone)]
pub struct AuditQuery {
    /// If set, only return events by this actor.
    pub actor_id: Option<Uuid>,
    /// If set, only return events of this kind.
    pub kind_filter: Option<AuditEventKind>,
    /// If set, only return events recorded at or after this time.
    pub since: Option<DateTime<Utc>>,
    /// If set, only return events recorded before this time.
    pub until: Option<DateTime<Utc>>,
    /// Maximum number of records to return (most-recent-first).
    pub limit: Option<usize>,
}

// ── Log ───────────────────────────────────────────────────────────────────────

/// Append-only structured audit log.
///
/// Thread-safe; in production this would flush to cave-etcd or object storage
/// for long-term retention and cross-cluster shipping.
#[derive(Debug, Default)]
pub struct AuditLog {
    records: Arc<RwLock<Vec<AuditRecord>>>,
}

impl AuditLog {
    /// Create a new empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an event. Returns the assigned sequence number.
    pub fn emit(&self, event: AuditEvent) -> u64 {
        let mut records = self.records.write().unwrap();
        let seq = records.len() as u64;
        records.push(AuditRecord {
            seq,
            recorded_at: Utc::now(),
            kind: event.kind,
            actor_id: event.actor_id,
            target: event.target,
            metadata: event.metadata,
        });
        seq
    }

    /// Query the audit log with optional filters.
    ///
    /// Results are returned oldest-first (ascending seq), then truncated by
    /// `limit` if provided.
    pub fn query(&self, q: AuditQuery) -> Vec<AuditRecord> {
        let records = self.records.read().unwrap();

        let mut result: Vec<AuditRecord> = records
            .iter()
            .filter(|r| {
                if let Some(actor) = &q.actor_id {
                    if &r.actor_id != actor {
                        return false;
                    }
                }
                if let Some(kind) = &q.kind_filter {
                    if &r.kind != kind {
                        return false;
                    }
                }
                if let Some(since) = q.since {
                    if r.recorded_at < since {
                        return false;
                    }
                }
                if let Some(until) = q.until {
                    if r.recorded_at >= until {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        if let Some(limit) = q.limit {
            result.truncate(limit);
        }
        result
    }

    /// Return the total number of events in the log (ignoring filters).
    pub fn total_count(&self) -> usize {
        self.records.read().unwrap().len()
    }

    /// Return events for a specific actor, sorted newest-first.
    pub fn events_for_actor(&self, actor_id: &Uuid) -> Vec<AuditRecord> {
        let mut records = self.query(AuditQuery {
            actor_id: Some(*actor_id),
            ..Default::default()
        });
        records.sort_by(|a, b| b.seq.cmp(&a.seq));
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_is_monotonically_increasing() {
        let log = AuditLog::new();
        let uid = Uuid::new_v4();
        for i in 0u64..5 {
            let seq = log.emit(AuditEvent {
                kind: AuditEventKind::CommandRun,
                actor_id: uid,
                target: "node".to_string(),
                metadata: serde_json::json!({}),
            });
            assert_eq!(seq, i);
        }
    }

    #[test]
    fn query_until_excludes_boundary() {
        let log = AuditLog::new();
        let uid = Uuid::new_v4();
        log.emit(AuditEvent {
            kind: AuditEventKind::SessionStart,
            actor_id: uid,
            target: "srv".to_string(),
            metadata: serde_json::json!({}),
        });
        let past = Utc::now() - chrono::Duration::hours(1);
        let records = log.query(AuditQuery {
            until: Some(past),
            ..Default::default()
        });
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn events_for_actor_newest_first() {
        let log = AuditLog::new();
        let uid = Uuid::new_v4();
        for _ in 0..3 {
            log.emit(AuditEvent {
                kind: AuditEventKind::AccessGranted,
                actor_id: uid,
                target: "node".to_string(),
                metadata: serde_json::json!({}),
            });
        }
        let events = log.events_for_actor(&uid);
        assert_eq!(events.len(), 3);
        assert!(events[0].seq > events[1].seq);
    }
}

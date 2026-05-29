// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for structured audit log.

use cave_pam::audit_log::{AuditEvent, AuditEventKind, AuditLog, AuditQuery};
use uuid::Uuid;
use chrono::{Duration, Utc};

fn emit_event(log: &AuditLog, kind: AuditEventKind, user_id: Uuid) {
    log.emit(AuditEvent {
        kind,
        actor_id: user_id,
        target: "server-01".to_string(),
        metadata: serde_json::json!({}),
    });
}

#[test]
fn test_emit_and_query_all() {
    let log = AuditLog::new();
    let uid = Uuid::new_v4();
    emit_event(&log, AuditEventKind::SessionStart, uid);
    emit_event(&log, AuditEventKind::SessionEnd, uid);

    let events = log.query(AuditQuery::default());
    assert_eq!(events.len(), 2);
}

#[test]
fn test_query_by_user() {
    let log = AuditLog::new();
    let u1 = Uuid::new_v4();
    let u2 = Uuid::new_v4();
    emit_event(&log, AuditEventKind::SessionStart, u1);
    emit_event(&log, AuditEventKind::AccessGranted, u1);
    emit_event(&log, AuditEventKind::SessionStart, u2);

    let events = log.query(AuditQuery { actor_id: Some(u1), ..Default::default() });
    assert_eq!(events.len(), 2);
    for e in &events {
        assert_eq!(e.actor_id, u1);
    }
}

#[test]
fn test_query_by_kind() {
    let log = AuditLog::new();
    let uid = Uuid::new_v4();
    emit_event(&log, AuditEventKind::SessionStart, uid);
    emit_event(&log, AuditEventKind::AccessDenied, uid);
    emit_event(&log, AuditEventKind::SessionStart, uid);

    let events = log.query(AuditQuery {
        kind_filter: Some(AuditEventKind::SessionStart),
        ..Default::default()
    });
    assert_eq!(events.len(), 2);
}

#[test]
fn test_query_with_limit() {
    let log = AuditLog::new();
    let uid = Uuid::new_v4();
    for _ in 0..10 {
        emit_event(&log, AuditEventKind::CommandRun, uid);
    }
    let events = log.query(AuditQuery { limit: Some(3), ..Default::default() });
    assert_eq!(events.len(), 3);
}

#[test]
fn test_query_since() {
    let log = AuditLog::new();
    let uid = Uuid::new_v4();
    emit_event(&log, AuditEventKind::SessionStart, uid);

    let future = Utc::now() + Duration::hours(1);
    let events = log.query(AuditQuery {
        since: Some(future),
        ..Default::default()
    });
    // No events after the future cutoff.
    assert_eq!(events.len(), 0);
}

#[test]
fn test_total_count() {
    let log = AuditLog::new();
    let uid = Uuid::new_v4();
    for _ in 0..5 {
        emit_event(&log, AuditEventKind::AccessGranted, uid);
    }
    assert_eq!(log.total_count(), 5);
}

#[test]
fn test_events_are_timestamped() {
    let log = AuditLog::new();
    let uid = Uuid::new_v4();
    emit_event(&log, AuditEventKind::SessionStart, uid);
    let events = log.query(AuditQuery::default());
    assert_eq!(events.len(), 1);
    // Timestamp must be recent (within last 5 seconds).
    let delta = (Utc::now() - events[0].recorded_at).num_seconds().abs();
    assert!(delta <= 5);
}

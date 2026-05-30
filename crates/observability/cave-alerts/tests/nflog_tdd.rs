// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD RED — 2026-05-30
//!
//! Line-port of the pure in-memory notification-log core from upstream
//! prometheus/alertmanager v0.26.0 `nflog/nflog.go`:
//!   - `stateKey` / `receiverKey` key derivation (lines 366-374)
//!   - `(*Log).Log` record-with-newer-timestamp-wins (lines 376-416)
//!   - `(*Log).Query` most-recent entry for (receiver, group_key) (lines 443-475)
//!   - `(*Log).GC` expiry by ExpiresAt (lines 419-440)
//!   - `state.merge` CRDT last-writer-wins reconciliation (lines 178-190)
//!
//! Persistence (Snapshot/loadSnapshot/Maintenance) and gossip broadcast stay
//! scope-cut; only the in-memory algorithm is ported.

use cave_alerts::nflog::{NflogError, NflogReceiver, NotificationLog, MeshEntry};
use chrono::{Duration, Utc};

fn recv(group: &str, integration: &str, idx: u32) -> NflogReceiver {
    NflogReceiver {
        group_name: group.into(),
        integration: integration.into(),
        idx,
    }
}

#[test]
fn test_receiver_key_format() {
    // receiverKey: "%s/%s/%d"
    let r = recv("team-X", "slack", 0);
    assert_eq!(r.receiver_key(), "team-X/slack/0");
}

#[test]
fn test_state_key_format() {
    // stateKey: "%s:%s" of group_key and receiver_key
    let r = recv("team-X", "slack", 0);
    assert_eq!(
        NotificationLog::state_key("gk1", &r),
        "gk1:team-X/slack/0"
    );
}

#[test]
fn test_log_then_query_returns_entry() {
    let mut log = NotificationLog::new(Duration::hours(120));
    let r = recv("default", "webhook", 0);
    log.log(&r, "gk1", vec![1, 2], vec![3], None)
        .expect("log ok");

    let entry = log.query(&r, "gk1").expect("found");
    assert_eq!(entry.firing_alerts, vec![1, 2]);
    assert_eq!(entry.resolved_alerts, vec![3]);
}

#[test]
fn test_query_unknown_returns_not_found() {
    let log = NotificationLog::new(Duration::hours(1));
    let r = recv("default", "webhook", 0);
    match log.query(&r, "missing") {
        Err(NflogError::NotFound) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn test_log_overwrites_only_with_newer_timestamp() {
    let mut log = NotificationLog::new(Duration::hours(120));
    let r = recv("default", "webhook", 0);
    let now = Utc::now();

    // First entry at `now`.
    log.log_at(&r, "gk1", vec![1], vec![], None, now).unwrap();
    // An older-timestamped write must NOT overwrite (clock-drift guard).
    log.log_at(&r, "gk1", vec![9, 9], vec![], None, now - Duration::minutes(5))
        .unwrap();

    let e = log.query(&r, "gk1").unwrap();
    assert_eq!(e.firing_alerts, vec![1], "older write must be ignored");

    // A newer-timestamped write overwrites.
    log.log_at(&r, "gk1", vec![7], vec![], None, now + Duration::minutes(5))
        .unwrap();
    let e2 = log.query(&r, "gk1").unwrap();
    assert_eq!(e2.firing_alerts, vec![7]);
}

#[test]
fn test_gc_expires_old_entries() {
    let mut log = NotificationLog::new(Duration::minutes(10));
    let r = recv("default", "webhook", 0);
    let base = Utc::now() - Duration::hours(1);
    // expires_at = base + retention(10m) => long in the past
    log.log_at(&r, "gk1", vec![1], vec![], None, base).unwrap();

    let removed = log.gc_at(Utc::now()).unwrap();
    assert_eq!(removed, 1);
    assert!(matches!(log.query(&r, "gk1"), Err(NflogError::NotFound)));
}

#[test]
fn test_gc_keeps_unexpired_entries() {
    let mut log = NotificationLog::new(Duration::hours(24));
    let r = recv("default", "webhook", 0);
    log.log(&r, "gk1", vec![1], vec![], None).unwrap();
    let removed = log.gc_at(Utc::now()).unwrap();
    assert_eq!(removed, 0);
    assert!(log.query(&r, "gk1").is_ok());
}

#[test]
fn test_explicit_expiry_shortens_retention() {
    // expiry > 0 && retention > expiry => use expiry
    let mut log = NotificationLog::new(Duration::hours(24));
    let r = recv("default", "webhook", 0);
    let now = Utc::now();
    log.log_at(&r, "gk1", vec![1], vec![], Some(Duration::minutes(1)), now)
        .unwrap();
    // After 2 minutes it should be GC-able even though retention is 24h.
    let removed = log.gc_at(now + Duration::minutes(2)).unwrap();
    assert_eq!(removed, 1);
}

#[test]
fn test_merge_last_writer_wins() {
    // state.merge: keep entry with later Timestamp; report whether merged.
    let mut log = NotificationLog::new(Duration::hours(120));
    let r = recv("default", "webhook", 0);
    let now = Utc::now();

    let older = MeshEntry {
        receiver: r.clone(),
        group_key: "gk1".into(),
        timestamp: now - Duration::minutes(1),
        firing_alerts: vec![1],
        resolved_alerts: vec![],
        expires_at: now + Duration::hours(1),
    };
    let newer = MeshEntry {
        receiver: r.clone(),
        group_key: "gk1".into(),
        timestamp: now,
        firing_alerts: vec![2],
        resolved_alerts: vec![],
        expires_at: now + Duration::hours(1),
    };

    assert!(log.merge(newer.clone(), now), "first merge inserts");
    // Older one must not overwrite and reports false.
    assert!(!log.merge(older, now), "older entry not merged");
    assert_eq!(log.query(&r, "gk1").unwrap().firing_alerts, vec![2]);
}

#[test]
fn test_merge_rejects_already_expired() {
    // merge returns false if ExpiresAt is before now.
    let mut log = NotificationLog::new(Duration::hours(1));
    let r = recv("default", "webhook", 0);
    let now = Utc::now();
    let expired = MeshEntry {
        receiver: r.clone(),
        group_key: "gk1".into(),
        timestamp: now,
        firing_alerts: vec![1],
        resolved_alerts: vec![],
        expires_at: now - Duration::seconds(1),
    };
    assert!(!log.merge(expired, now), "expired entry not merged");
    assert!(matches!(log.query(&r, "gk1"), Err(NflogError::NotFound)));
}

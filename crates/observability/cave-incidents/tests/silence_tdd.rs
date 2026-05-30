// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD RED test for the incident-group silence state machine and the
//! computed status-precedence resolution.
//!
//! Line-ports grafana/oncall v1.10.0:
//!   engine/apps/alerts/models/alert_group.py ::
//!     - `status` property  (resolved > acknowledged > silenced > new)
//!     - `is_silenced_forever`  (silenced && silenced_until is None)
//!     - `is_silenced_for_period`  (silenced && silenced_until is Some)
//!     - `silence` / `un_silence`
//!
//! This is the incident-side silenced state (whether THIS incident group is
//! muted, and until when). The Alertmanager routing-tree / inhibit-rule silences
//! remain in cave-alerts.

use cave_incidents::silence::{ComputedStatus, GroupSilenceState};
use chrono::{Duration, Utc};

#[test]
fn test_new_group_status_is_new() {
    let s = GroupSilenceState::default();
    assert_eq!(s.status(), ComputedStatus::New);
    assert!(!s.is_silenced_forever());
    assert!(!s.is_silenced_for_period());
}

#[test]
fn test_silence_forever_when_no_until() {
    let mut s = GroupSilenceState::default();
    s.silence(None); // silenced forever
    assert!(s.is_silenced_forever());
    assert!(!s.is_silenced_for_period());
    assert_eq!(s.status(), ComputedStatus::Silenced);
}

#[test]
fn test_silence_for_period_when_until_set() {
    let mut s = GroupSilenceState::default();
    let until = Utc::now() + Duration::hours(2);
    s.silence(Some(until));
    assert!(!s.is_silenced_forever());
    assert!(s.is_silenced_for_period());
    assert_eq!(s.silenced_until, Some(until));
    assert_eq!(s.status(), ComputedStatus::Silenced);
}

#[test]
fn test_un_silence_clears_all_fields() {
    let mut s = GroupSilenceState::default();
    s.silence(Some(Utc::now() + Duration::hours(1)));
    assert!(s.silenced);
    s.un_silence();
    assert!(!s.silenced);
    assert!(s.silenced_until.is_none());
    assert!(s.restarted_at.is_some(), "un_silence stamps restarted_at");
    assert_eq!(s.status(), ComputedStatus::New);
}

#[test]
fn test_status_precedence_resolved_over_everything() {
    // resolved beats acknowledged beats silenced beats new
    let mut s = GroupSilenceState::default();
    s.silence(None);
    s.acknowledged = true;
    s.resolved = true;
    assert_eq!(s.status(), ComputedStatus::Resolved);
}

#[test]
fn test_status_precedence_acknowledged_over_silenced() {
    let mut s = GroupSilenceState::default();
    s.silence(None);
    s.acknowledged = true;
    assert_eq!(s.status(), ComputedStatus::Acknowledged);
}

#[test]
fn test_silence_is_idempotent() {
    // upstream `silence` only mutates when not already silenced; calling twice
    // must not clobber the original silenced_until.
    let mut s = GroupSilenceState::default();
    let until = Utc::now() + Duration::hours(3);
    s.silence(Some(until));
    s.silence(Some(Utc::now() + Duration::hours(99)));
    assert_eq!(s.silenced_until, Some(until));
}

#[test]
fn test_expired_period_silence_is_no_longer_active() {
    // a for-period silence whose `silenced_until` is in the past is effectively
    // expired (the unsilence task would have fired); `is_active_at` reflects that.
    let mut s = GroupSilenceState::default();
    let past = Utc::now() - Duration::hours(1);
    s.silence(Some(past));
    assert!(!s.is_active_at(Utc::now()), "past silenced_until -> not active");
    // a forever silence is always active until explicitly un-silenced
    let mut f = GroupSilenceState::default();
    f.silence(None);
    assert!(f.is_active_at(Utc::now()));
}

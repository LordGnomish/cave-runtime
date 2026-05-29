// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for session recording and event audit trail.

use cave_pam::session_recorder::{
    SessionEvent, SessionEventKind, SessionRecorder, SessionRecording,
};
use uuid::Uuid;

#[test]
fn test_recording_lifecycle_complete() {
    let recorder = SessionRecorder::new();
    let session_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    recorder.record(session_id, SessionEvent {
        kind: SessionEventKind::SessionStart,
        actor: user_id,
        data: serde_json::json!({"target": "srv-01", "protocol": "ssh"}),
    });

    recorder.record(session_id, SessionEvent {
        kind: SessionEventKind::CommandRun,
        actor: user_id,
        data: serde_json::json!({"cmd": "ls -la /etc"}),
    });

    recorder.record(session_id, SessionEvent {
        kind: SessionEventKind::CommandRun,
        actor: user_id,
        data: serde_json::json!({"cmd": "cat /etc/passwd"}),
    });

    recorder.record(session_id, SessionEvent {
        kind: SessionEventKind::SessionEnd,
        actor: user_id,
        data: serde_json::json!({"exit_code": 0}),
    });

    let recording = recorder.get_recording(&session_id).expect("should have recording");
    assert_eq!(recording.events.len(), 4);
    assert_eq!(recording.session_id, session_id);
    assert!(recording.ended_at.is_some());
}

#[test]
fn test_recording_event_count() {
    let recorder = SessionRecorder::new();
    let sid = Uuid::new_v4();
    let uid = Uuid::new_v4();

    for i in 0..5 {
        recorder.record(sid, SessionEvent {
            kind: SessionEventKind::Print,
            actor: uid,
            data: serde_json::json!({"line": i, "text": "output"}),
        });
    }

    let rec = recorder.get_recording(&sid).unwrap();
    assert_eq!(rec.events.len(), 5);
}

#[test]
fn test_get_nonexistent_recording_returns_none() {
    let recorder = SessionRecorder::new();
    assert!(recorder.get_recording(&Uuid::new_v4()).is_none());
}

#[test]
fn test_list_recordings_for_user() {
    let recorder = SessionRecorder::new();
    let user_a = Uuid::new_v4();
    let user_b = Uuid::new_v4();
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();

    recorder.record(sid_a, SessionEvent {
        kind: SessionEventKind::SessionStart,
        actor: user_a,
        data: serde_json::json!({}),
    });
    recorder.record(sid_b, SessionEvent {
        kind: SessionEventKind::SessionStart,
        actor: user_b,
        data: serde_json::json!({}),
    });

    let recs_a = recorder.list_for_user(&user_a);
    let recs_b = recorder.list_for_user(&user_b);
    assert_eq!(recs_a.len(), 1);
    assert_eq!(recs_b.len(), 1);
    assert_eq!(recs_a[0].session_id, sid_a);
}

#[test]
fn test_session_start_sets_started_at() {
    let recorder = SessionRecorder::new();
    let sid = Uuid::new_v4();
    let uid = Uuid::new_v4();

    recorder.record(sid, SessionEvent {
        kind: SessionEventKind::SessionStart,
        actor: uid,
        data: serde_json::json!({}),
    });
    let rec = recorder.get_recording(&sid).unwrap();
    assert!(rec.started_at.is_some());
    assert!(rec.ended_at.is_none());
}

#[test]
fn test_session_end_sets_ended_at() {
    let recorder = SessionRecorder::new();
    let sid = Uuid::new_v4();
    let uid = Uuid::new_v4();

    recorder.record(sid, SessionEvent { kind: SessionEventKind::SessionStart, actor: uid, data: serde_json::json!({}) });
    recorder.record(sid, SessionEvent { kind: SessionEventKind::SessionEnd, actor: uid, data: serde_json::json!({"exit": 0}) });

    let rec = recorder.get_recording(&sid).unwrap();
    assert!(rec.ended_at.is_some());
}

#[test]
fn test_recording_serializes_to_json() {
    let recorder = SessionRecorder::new();
    let sid = Uuid::new_v4();
    let uid = Uuid::new_v4();
    recorder.record(sid, SessionEvent {
        kind: SessionEventKind::SessionStart,
        actor: uid,
        data: serde_json::json!({}),
    });
    let rec = recorder.get_recording(&sid).unwrap();
    let json = serde_json::to_string(&rec).expect("serialization must succeed");
    assert!(json.contains("session_id"));
}

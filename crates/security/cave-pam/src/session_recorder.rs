// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Session recording subsystem.
//!
//! Records structured audit events for each privileged session, providing a
//! complete audit trail of commands run, data printed, and lifecycle events.
//! Modelled after Teleport's Enhanced Session Recording (ESR) event model.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ── Event types ───────────────────────────────────────────────────────────────

/// Kinds of events that can be recorded in a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    /// Session has started (first event).
    SessionStart,
    /// Session has ended (last event).
    SessionEnd,
    /// A shell command was executed.
    CommandRun,
    /// Terminal output was printed.
    Print,
    /// A file was transferred (SFTP/SCP).
    FileTransfer,
    /// A port-forward was opened.
    PortForward,
    /// A sub-session was created (e.g., kubectl exec).
    SubSession,
    /// Session was forcibly terminated by an administrator.
    ForceTerminate,
    /// User identity was re-verified (MFA touch).
    MfaVerify,
}

/// A single recorded event within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Type of event.
    pub kind: SessionEventKind,
    /// User who triggered the event.
    pub actor: Uuid,
    /// Event-specific structured payload.
    pub data: serde_json::Value,
}

/// An internal event with a monotonic sequence number and wall-clock timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampedEvent {
    /// Monotonically increasing event sequence within the session.
    pub seq: u64,
    /// Wall-clock time when the event was recorded.
    pub recorded_at: DateTime<Utc>,
    /// The event itself.
    pub event: SessionEvent,
}

// ── Recording ─────────────────────────────────────────────────────────────────

/// The full recording for one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecording {
    /// Session identifier.
    pub session_id: Uuid,
    /// User that started the session (derived from first SessionStart event).
    pub user_id: Option<Uuid>,
    /// When the first event was recorded.
    pub started_at: Option<DateTime<Utc>>,
    /// When the SessionEnd event was recorded (None if still active).
    pub ended_at: Option<DateTime<Utc>>,
    /// All events in sequence order.
    pub events: Vec<TimestampedEvent>,
}

impl SessionRecording {
    fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            user_id: None,
            started_at: None,
            ended_at: None,
            events: Vec::new(),
        }
    }

    /// Append a new event, assigning the next sequence number.
    fn append(&mut self, event: SessionEvent) {
        let seq = self.events.len() as u64;
        let now = Utc::now();

        // Track lifecycle timestamps.
        match &event.kind {
            SessionEventKind::SessionStart => {
                if self.started_at.is_none() {
                    self.started_at = Some(now);
                    self.user_id = Some(event.actor);
                }
            }
            SessionEventKind::SessionEnd => {
                self.ended_at = Some(now);
            }
            _ => {}
        }

        self.events.push(TimestampedEvent {
            seq,
            recorded_at: now,
            event,
        });
    }

    /// Return true if this session has been concluded (SessionEnd received).
    pub fn is_finished(&self) -> bool {
        self.ended_at.is_some()
    }

    /// Total event count.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Duration of the session in seconds; None if not yet ended.
    pub fn duration_secs(&self) -> Option<i64> {
        match (self.started_at, self.ended_at) {
            (Some(start), Some(end)) => Some((end - start).num_seconds()),
            _ => None,
        }
    }
}

// ── Recorder ─────────────────────────────────────────────────────────────────

/// Thread-safe session recorder.
///
/// In production the event stream would be flushed to cave-etcd / an S3-
/// compatible store for long-term retention; this in-memory implementation
/// covers the core recording API.
#[derive(Debug, Default)]
pub struct SessionRecorder {
    recordings: Arc<RwLock<HashMap<Uuid, SessionRecording>>>,
}

impl SessionRecorder {
    /// Create a new recorder with no sessions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an event for the given session.
    ///
    /// If no recording exists for `session_id`, one is created automatically.
    pub fn record(&self, session_id: Uuid, event: SessionEvent) {
        let mut map = self.recordings.write().unwrap();
        let recording = map
            .entry(session_id)
            .or_insert_with(|| SessionRecording::new(session_id));
        recording.append(event);
    }

    /// Retrieve the full recording for a session.
    pub fn get_recording(&self, session_id: &Uuid) -> Option<SessionRecording> {
        self.recordings.read().unwrap().get(session_id).cloned()
    }

    /// List all recordings where the first actor matches `user_id`.
    pub fn list_for_user(&self, user_id: &Uuid) -> Vec<SessionRecording> {
        self.recordings
            .read()
            .unwrap()
            .values()
            .filter(|r| r.user_id.as_ref() == Some(user_id))
            .cloned()
            .collect()
    }

    /// Return the count of currently active (not-finished) sessions.
    pub fn active_count(&self) -> usize {
        self.recordings
            .read()
            .unwrap()
            .values()
            .filter(|r| !r.is_finished())
            .count()
    }

    /// Return only recordings that have ended.
    pub fn list_completed(&self) -> Vec<SessionRecording> {
        self.recordings
            .read()
            .unwrap()
            .values()
            .filter(|r| r.is_finished())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_sequence_is_monotonic() {
        let recorder = SessionRecorder::new();
        let sid = Uuid::new_v4();
        let uid = Uuid::new_v4();
        for _ in 0..3 {
            recorder.record(
                sid,
                SessionEvent {
                    kind: SessionEventKind::Print,
                    actor: uid,
                    data: serde_json::json!({}),
                },
            );
        }
        let rec = recorder.get_recording(&sid).unwrap();
        let seqs: Vec<u64> = rec.events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn active_count_decrements_on_end() {
        let recorder = SessionRecorder::new();
        let sid = Uuid::new_v4();
        let uid = Uuid::new_v4();
        recorder.record(
            sid,
            SessionEvent {
                kind: SessionEventKind::SessionStart,
                actor: uid,
                data: serde_json::json!({}),
            },
        );
        assert_eq!(recorder.active_count(), 1);
        recorder.record(
            sid,
            SessionEvent {
                kind: SessionEventKind::SessionEnd,
                actor: uid,
                data: serde_json::json!({}),
            },
        );
        assert_eq!(recorder.active_count(), 0);
    }

    #[test]
    fn duration_secs_returns_none_for_unfinished() {
        let recorder = SessionRecorder::new();
        let sid = Uuid::new_v4();
        let uid = Uuid::new_v4();
        recorder.record(
            sid,
            SessionEvent {
                kind: SessionEventKind::SessionStart,
                actor: uid,
                data: serde_json::json!({}),
            },
        );
        let rec = recorder.get_recording(&sid).unwrap();
        assert!(rec.duration_secs().is_none());
    }
}

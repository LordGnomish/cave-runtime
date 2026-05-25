// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Session event log.
//!
//! Hermes' `agent/credential_sources.py` and `agent/run_agent.py` together
//! own the session state: a monotonically increasing event log that
//! records user turns, model responses, tool calls, and recovery events.
//! We extract the log itself here and leave the credential plumbing for
//! the cave-vault integration sprint.
//!
//! The log is append-only, in-memory by default, with an optional JSON-Lines
//! sink for crash recovery. Each [`Event`] carries an ISO-8601 timestamp,
//! an [`EventKind`], a small free-form payload, and a stable
//! [`Event::id`] (UUID v4).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::error::HermesError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    UserTurn,
    AssistantTurn,
    ToolCall,
    ToolResult,
    PlanCreated,
    Checkpoint,
    Recall,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub kind: EventKind,
    pub at: String,
    pub payload: serde_json::Value,
}

impl Event {
    pub fn new(kind: EventKind, payload: serde_json::Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            at: chrono::Utc::now().to_rfc3339(),
            payload,
        }
    }
}

/// Append-only event log. Cheap mutex over a Vec; if we ever need to
/// stream this over the network we'll swap in a tokio::broadcast.
#[derive(Debug)]
pub struct SessionStore {
    events: Arc<RwLock<Vec<Event>>>,
    sink: Arc<RwLock<Option<PathBuf>>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
            sink: Arc::new(RwLock::new(None)),
        }
    }

    /// Install a JSONL sink. Every subsequent `append` flushes a line.
    /// The file is created if missing and opened in append mode.
    pub fn with_sink(self, path: impl Into<PathBuf>) -> Self {
        *self.sink.write() = Some(path.into());
        self
    }

    pub fn append(&self, event: Event) -> crate::error::Result<()> {
        if let Some(path) = self.sink.read().as_ref() {
            let body = serde_json::to_string(&event)?;
            let mut f = OpenOptions::new().create(true).append(true).open(path)?;
            writeln!(f, "{body}")?;
        }
        self.events.write().push(event);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.events.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.read().is_empty()
    }

    pub fn snapshot(&self) -> Vec<Event> {
        self.events.read().clone()
    }

    /// Replay a JSONL log into a fresh store. Used on crash recovery.
    pub fn replay(path: impl AsRef<Path>) -> crate::error::Result<Self> {
        let path = path.as_ref();
        let store = SessionStore::new();
        if !path.exists() {
            return Ok(store);
        }
        let raw = std::fs::read_to_string(path)?;
        for (lineno, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let ev: Event = serde_json::from_str(line).map_err(|e| {
                HermesError::SessionCorrupted(format!("{}:{} {e}", path.display(), lineno + 1))
            })?;
            store.events.write().push(ev);
        }
        Ok(store)
    }

    /// Filter the log by kind. Cheap convenience for tests + dashboards.
    pub fn of_kind(&self, kind: EventKind) -> Vec<Event> {
        self.events
            .read()
            .iter()
            .filter(|e| e.kind == kind)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn fresh_store_is_empty() {
        let s = SessionStore::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn append_grows_log() {
        let s = SessionStore::new();
        s.append(Event::new(
            EventKind::UserTurn,
            serde_json::json!({"text": "hi"}),
        ))
        .unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.snapshot()[0].kind, EventKind::UserTurn);
    }

    #[test]
    fn of_kind_filters() {
        let s = SessionStore::new();
        s.append(Event::new(EventKind::UserTurn, serde_json::json!({})))
            .unwrap();
        s.append(Event::new(EventKind::AssistantTurn, serde_json::json!({})))
            .unwrap();
        s.append(Event::new(EventKind::UserTurn, serde_json::json!({})))
            .unwrap();
        assert_eq!(s.of_kind(EventKind::UserTurn).len(), 2);
        assert_eq!(s.of_kind(EventKind::AssistantTurn).len(), 1);
    }

    #[test]
    fn sink_writes_jsonl_and_replay_restores() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let s = SessionStore::new().with_sink(&path);
        for i in 0..3 {
            s.append(Event::new(EventKind::ToolCall, serde_json::json!({"n": i})))
                .unwrap();
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 3);
        let restored = SessionStore::replay(&path).unwrap();
        assert_eq!(restored.len(), 3);
    }

    #[test]
    fn replay_missing_file_yields_empty_store() {
        let dir = tempdir().unwrap();
        let s = SessionStore::replay(dir.path().join("nope.jsonl")).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn replay_rejects_corrupt_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.jsonl");
        std::fs::write(&path, "this-is-not-json\n").unwrap();
        let err = SessionStore::replay(&path).unwrap_err();
        assert!(matches!(err, HermesError::SessionCorrupted(_)));
    }

    #[test]
    fn event_id_is_unique_per_event() {
        let a = Event::new(EventKind::UserTurn, serde_json::Value::Null);
        let b = Event::new(EventKind::UserTurn, serde_json::Value::Null);
        assert_ne!(a.id, b.id);
    }
}

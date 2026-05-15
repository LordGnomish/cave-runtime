//! `GAP_OPENED` event + sinks.
//!
//! The daemon emits one event per upstream whose `tag_name` has
//! moved past our local pin. Event payload includes everything a
//! downstream dispatcher needs to either:
//!
//! * page an operator (Slack webhook), or
//! * draft a port prompt for the Qwen / Opus loop (Charter v2 — out
//!   of scope here).
//!
//! Default sink is JSONL append-only at
//! `<data_dir>/watchd/events.jsonl`. The trait abstracts over
//! transport so an HTTP webhook or NATS subject sink can drop in
//! later.

use crate::changelog::Changelog;
use crate::diff::Severity;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmitError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// One emitted gap. Stable wire schema — the dispatcher reads this
/// from JSONL so adding a field with `#[serde(default)]` is safe but
/// removing fields is a breaking change.
///
/// `Eq` is omitted because `current_parity_ratio: Option<f64>` makes
/// total equality unsafe; `PartialEq` covers the test paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapEvent {
    pub event_id: String,
    pub at: DateTime<Utc>,
    /// Event class. Currently always `"GAP_OPENED"`; reserved as
    /// String (not &'static) so a future `GAP_CLOSED` /
    /// `RELEASE_BLOCKED` can be added without a wire change.
    pub kind: String,
    pub cave_module: String,
    pub github_repo: String,
    pub previous_pin: Option<String>,
    pub latest_tag: String,
    pub severity: Severity,
    pub gap_age_seconds: Option<i64>,
    /// `parity_ratio` of `cave_module` at the moment the event fired,
    /// read from the live `parity-index.json` by the daemon.
    pub current_parity_ratio: Option<f64>,
    pub changelog: Changelog,
}

impl GapEvent {
    pub fn new(
        cave_module: impl Into<String>,
        github_repo: impl Into<String>,
        previous_pin: Option<String>,
        latest_tag: impl Into<String>,
        severity: Severity,
        gap_age_seconds: Option<i64>,
        current_parity_ratio: Option<f64>,
        changelog: Changelog,
        at: DateTime<Utc>,
    ) -> Self {
        let cave_module = cave_module.into();
        Self {
            event_id: format!(
                "GAP-{}-{}",
                at.format("%Y%m%dT%H%M%SZ"),
                shorthash(&cave_module)
            ),
            at,
            kind: "GAP_OPENED".to_string(),
            cave_module,
            github_repo: github_repo.into(),
            previous_pin,
            latest_tag: latest_tag.into(),
            severity,
            gap_age_seconds,
            current_parity_ratio,
            changelog,
        }
    }
}

/// Trait abstraction so the daemon can swap JSONL ↔ webhook ↔ NATS.
pub trait GapEventSink: Send + Sync {
    fn emit(&self, event: &GapEvent) -> Result<(), EmitError>;
}

/// JSONL append-only sink. Path is created on first emit.
pub struct JsonlSink {
    pub path: PathBuf,
}

impl JsonlSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Honours `$CAVE_WATCHD_EVENTS` then falls back to
    /// `<dirs::data_dir>/cave-runtime/watchd/events.jsonl`.
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("CAVE_WATCHD_EVENTS") {
            return PathBuf::from(p);
        }
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("cave-runtime").join("watchd").join("events.jsonl")
    }
}

impl GapEventSink for JsonlSink {
    fn emit(&self, event: &GapEvent) -> Result<(), EmitError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(event)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        Ok(())
    }
}

/// Convenience: emit one event via the given sink. The function
/// captures the call-site so a single line in `daemon.rs` can build
/// the event + send it.
pub fn emit(sink: &dyn GapEventSink, event: &GapEvent) -> Result<(), EmitError> {
    sink.emit(event)
}

/// Read every event from a JSONL file, newest first. Skips malformed
/// lines silently — this is a "read-side" helper for the portal
/// dashboard; malformed lines are reported to telemetry elsewhere.
pub fn read_events(path: &Path) -> Result<Vec<GapEvent>, EmitError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(EmitError::Io(e)),
    };
    let mut out: Vec<GapEvent> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(e) = serde_json::from_str::<GapEvent>(trimmed) {
            out.push(e);
        }
    }
    out.reverse();
    Ok(out)
}

fn shorthash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:x}", h.finish() & 0xFFFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changelog::{ChangeKind, ChangelogEntry};

    fn ts() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-13T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_event(name: &str) -> GapEvent {
        let cl = Changelog {
            entries: vec![ChangelogEntry {
                kind: ChangeKind::Added,
                description: "feature x".into(),
                breaking: false,
            }],
        };
        GapEvent::new(
            name,
            "x/y",
            Some("v1.0.0".into()),
            "v1.1.0",
            Severity::Minor,
            Some(7200),
            Some(0.8),
            cl,
            ts(),
        )
    }

    #[test]
    fn jsonl_sink_appends_one_line_per_event() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        sink.emit(&sample_event("cave-a")).unwrap();
        sink.emit(&sample_event("cave-b")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        // Each line round-trips through serde.
        for line in lines {
            let _: GapEvent = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn read_events_returns_newest_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        sink.emit(&sample_event("cave-a")).unwrap();
        sink.emit(&sample_event("cave-b")).unwrap();
        let got = read_events(&path).unwrap();
        assert_eq!(got.len(), 2);
        // Newest first → "cave-b" emitted last → first in result.
        assert_eq!(got[0].cave_module, "cave-b");
        assert_eq!(got[1].cave_module, "cave-a");
    }

    #[test]
    fn read_events_missing_file_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("never-exists.jsonl");
        let got = read_events(&path).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn read_events_skips_malformed_lines() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        sink.emit(&sample_event("cave-a")).unwrap();
        // Append a garbage line.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{not json\n").unwrap();
        sink.emit(&sample_event("cave-b")).unwrap();
        let got = read_events(&path).unwrap();
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn event_id_includes_timestamp_and_module_hash() {
        let e = sample_event("cave-x");
        assert!(e.event_id.starts_with("GAP-2026"));
        assert!(e.event_id.contains("T143000Z"));
    }

    #[test]
    fn default_path_honors_env() {
        unsafe {
            std::env::set_var("CAVE_WATCHD_EVENTS", "/tmp/__cave_watchd_evt/x.jsonl");
        }
        let p = JsonlSink::default_path();
        assert_eq!(p, PathBuf::from("/tmp/__cave_watchd_evt/x.jsonl"));
        unsafe {
            std::env::remove_var("CAVE_WATCHD_EVENTS");
        }
    }
}

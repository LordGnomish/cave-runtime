//! Image pull progress tracking.
//!
//! Mirrors containerd's `pkg/cri/server/image_pull.go` ProgressTracker
//! and `progress.Reader` plumbing — each layer download emits start /
//! progress / complete events, plus an overall `Done` when the manifest
//! is fully resolved. The frontend (kubelet, CLI) subscribes to the
//! per-pull stream to render a progress bar.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

/// One progress event for an image pull.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PullEvent {
    /// Pull was registered. `image_ref` is the user-facing reference.
    Started {
        pull_id: Uuid,
        image_ref: String,
    },
    /// Manifest fetched and parsed; we now know the total work.
    ManifestFetched {
        pull_id: Uuid,
        layer_count: u32,
        total_bytes: u64,
    },
    /// One layer's download has begun.
    LayerStarted {
        pull_id: Uuid,
        digest: String,
        total_bytes: u64,
    },
    /// One layer's bytes have advanced.
    LayerProgress {
        pull_id: Uuid,
        digest: String,
        downloaded_bytes: u64,
    },
    /// One layer is fully downloaded and verified.
    LayerComplete {
        pull_id: Uuid,
        digest: String,
    },
    /// All layers finished and the image is registered locally.
    Completed {
        pull_id: Uuid,
        image_ref: String,
        total_bytes: u64,
    },
    /// Pull aborted (network error, digest mismatch, manifest invalid…).
    Failed {
        pull_id: Uuid,
        image_ref: String,
        reason: String,
    },
}

impl PullEvent {
    pub fn pull_id(&self) -> Uuid {
        match self {
            PullEvent::Started { pull_id, .. }
            | PullEvent::ManifestFetched { pull_id, .. }
            | PullEvent::LayerStarted { pull_id, .. }
            | PullEvent::LayerProgress { pull_id, .. }
            | PullEvent::LayerComplete { pull_id, .. }
            | PullEvent::Completed { pull_id, .. }
            | PullEvent::Failed { pull_id, .. } => *pull_id,
        }
    }
}

/// Aggregated state of a single pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullState {
    pub pull_id: Uuid,
    pub image_ref: String,
    pub started_at: DateTime<Utc>,
    pub layer_count: u32,
    pub layers_complete: u32,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub status: PullStatus,
    /// Last update timestamp; lets the frontend infer staleness.
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullStatus {
    Started,
    InProgress,
    Completed,
    Failed,
}

impl PullState {
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 { 0.0 } else {
            self.downloaded_bytes as f64 / self.total_bytes as f64
        }
    }
}

/// In-process pull-progress tracker. Holds the full event log per pull
/// plus an aggregated `PullState` summary suitable for `kubectl describe`
/// or `crictl pull --quiet=false`.
#[derive(Debug, Default)]
pub struct PullProgressTracker {
    events: RwLock<HashMap<Uuid, Vec<PullEvent>>>,
    state: RwLock<HashMap<Uuid, PullState>>,
    /// Per-layer downloaded byte counters so layer progress sums into
    /// the aggregate downloaded_bytes correctly.
    layer_bytes: RwLock<HashMap<(Uuid, String), u64>>,
}

impl PullProgressTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin tracking a new pull and emit `Started`.
    pub fn start(&self, image_ref: &str) -> Uuid {
        let pull_id = Uuid::new_v4();
        let now = Utc::now();
        let state = PullState {
            pull_id,
            image_ref: image_ref.to_string(),
            started_at: now,
            layer_count: 0,
            layers_complete: 0,
            total_bytes: 0,
            downloaded_bytes: 0,
            status: PullStatus::Started,
            updated_at: now,
        };
        self.state.write().unwrap().insert(pull_id, state);
        self.record(PullEvent::Started { pull_id, image_ref: image_ref.to_string() });
        pull_id
    }

    pub fn manifest_fetched(&self, pull_id: Uuid, layer_count: u32, total_bytes: u64) {
        if let Some(s) = self.state.write().unwrap().get_mut(&pull_id) {
            s.layer_count = layer_count;
            s.total_bytes = total_bytes;
            s.status = PullStatus::InProgress;
            s.updated_at = Utc::now();
        }
        self.record(PullEvent::ManifestFetched { pull_id, layer_count, total_bytes });
    }

    pub fn layer_started(&self, pull_id: Uuid, digest: &str, total_bytes: u64) {
        self.record(PullEvent::LayerStarted {
            pull_id,
            digest: digest.to_string(),
            total_bytes,
        });
    }

    pub fn layer_progress(&self, pull_id: Uuid, digest: &str, downloaded_bytes: u64) {
        let key = (pull_id, digest.to_string());
        let mut by_layer = self.layer_bytes.write().unwrap();
        let prev = by_layer.insert(key, downloaded_bytes).unwrap_or(0);
        let delta = downloaded_bytes.saturating_sub(prev);
        if delta > 0 {
            if let Some(s) = self.state.write().unwrap().get_mut(&pull_id) {
                s.downloaded_bytes = s.downloaded_bytes.saturating_add(delta);
                s.updated_at = Utc::now();
            }
        }
        self.record(PullEvent::LayerProgress {
            pull_id,
            digest: digest.to_string(),
            downloaded_bytes,
        });
    }

    pub fn layer_complete(&self, pull_id: Uuid, digest: &str) {
        if let Some(s) = self.state.write().unwrap().get_mut(&pull_id) {
            s.layers_complete = s.layers_complete.saturating_add(1);
            s.updated_at = Utc::now();
        }
        self.record(PullEvent::LayerComplete { pull_id, digest: digest.to_string() });
    }

    pub fn completed(&self, pull_id: Uuid, image_ref: &str) {
        if let Some(s) = self.state.write().unwrap().get_mut(&pull_id) {
            s.status = PullStatus::Completed;
            // Snap downloaded to total to avoid floating-point oddities.
            s.downloaded_bytes = s.total_bytes;
            s.updated_at = Utc::now();
        }
        let total = self.state.read().unwrap().get(&pull_id).map(|s| s.total_bytes).unwrap_or(0);
        self.record(PullEvent::Completed {
            pull_id,
            image_ref: image_ref.to_string(),
            total_bytes: total,
        });
    }

    pub fn failed(&self, pull_id: Uuid, image_ref: &str, reason: &str) {
        if let Some(s) = self.state.write().unwrap().get_mut(&pull_id) {
            s.status = PullStatus::Failed;
            s.updated_at = Utc::now();
        }
        self.record(PullEvent::Failed {
            pull_id,
            image_ref: image_ref.to_string(),
            reason: reason.to_string(),
        });
    }

    pub fn state(&self, pull_id: Uuid) -> Option<PullState> {
        self.state.read().unwrap().get(&pull_id).cloned()
    }

    pub fn events(&self, pull_id: Uuid) -> Vec<PullEvent> {
        self.events.read().unwrap().get(&pull_id).cloned().unwrap_or_default()
    }

    pub fn list(&self) -> Vec<PullState> {
        self.state.read().unwrap().values().cloned().collect()
    }

    fn record(&self, event: PullEvent) {
        let id = event.pull_id();
        self.events.write().unwrap().entry(id).or_default().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PullEvent ────────────────────────────────────────────────────────────

    #[test]
    fn pull_id_returns_consistent_uuid() {
        let id = Uuid::new_v4();
        for e in [
            PullEvent::Started { pull_id: id, image_ref: "x".into() },
            PullEvent::ManifestFetched { pull_id: id, layer_count: 1, total_bytes: 10 },
            PullEvent::LayerStarted { pull_id: id, digest: "d".into(), total_bytes: 5 },
            PullEvent::LayerProgress { pull_id: id, digest: "d".into(), downloaded_bytes: 3 },
            PullEvent::LayerComplete { pull_id: id, digest: "d".into() },
            PullEvent::Completed { pull_id: id, image_ref: "x".into(), total_bytes: 10 },
            PullEvent::Failed { pull_id: id, image_ref: "x".into(), reason: "r".into() },
        ] {
            assert_eq!(e.pull_id(), id);
        }
    }

    #[test]
    fn pull_event_serializes_with_tag() {
        let e = PullEvent::LayerComplete {
            pull_id: Uuid::nil(),
            digest: "sha256:abc".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"LayerComplete\""));
        let back: PullEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    // ── Tracker happy path ──────────────────────────────────────────────────

    #[test]
    fn start_creates_state_and_emits_started_event() {
        let t = PullProgressTracker::new();
        let id = t.start("nginx:latest");
        let state = t.state(id).unwrap();
        assert_eq!(state.image_ref, "nginx:latest");
        assert_eq!(state.status, PullStatus::Started);
        let events = t.events(id);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], PullEvent::Started { .. }));
    }

    #[test]
    fn manifest_fetched_advances_to_in_progress() {
        let t = PullProgressTracker::new();
        let id = t.start("a");
        t.manifest_fetched(id, 3, 1000);
        let s = t.state(id).unwrap();
        assert_eq!(s.layer_count, 3);
        assert_eq!(s.total_bytes, 1000);
        assert_eq!(s.status, PullStatus::InProgress);
    }

    #[test]
    fn layer_progress_accumulates_downloaded_bytes() {
        let t = PullProgressTracker::new();
        let id = t.start("a");
        t.manifest_fetched(id, 2, 200);
        t.layer_started(id, "d1", 100);
        t.layer_progress(id, "d1", 30);
        t.layer_progress(id, "d1", 60);
        t.layer_progress(id, "d1", 100);
        t.layer_complete(id, "d1");
        let s = t.state(id).unwrap();
        // The layer was driven to 100 bytes downloaded; aggregate matches.
        assert_eq!(s.downloaded_bytes, 100);
        assert_eq!(s.layers_complete, 1);
    }

    #[test]
    fn layer_progress_for_two_layers_sums_correctly() {
        let t = PullProgressTracker::new();
        let id = t.start("a");
        t.manifest_fetched(id, 2, 200);
        t.layer_progress(id, "d1", 40);
        t.layer_progress(id, "d2", 70);
        let s = t.state(id).unwrap();
        assert_eq!(s.downloaded_bytes, 110);
    }

    #[test]
    fn completed_snaps_downloaded_to_total_and_marks_status() {
        let t = PullProgressTracker::new();
        let id = t.start("a");
        t.manifest_fetched(id, 1, 500);
        t.layer_progress(id, "d", 300);
        t.completed(id, "a");
        let s = t.state(id).unwrap();
        assert_eq!(s.status, PullStatus::Completed);
        assert_eq!(s.downloaded_bytes, 500);
    }

    #[test]
    fn failed_sets_status_and_records_reason() {
        let t = PullProgressTracker::new();
        let id = t.start("a");
        t.failed(id, "a", "manifest 401");
        let s = t.state(id).unwrap();
        assert_eq!(s.status, PullStatus::Failed);
        let events = t.events(id);
        match events.last().unwrap() {
            PullEvent::Failed { reason, .. } => assert!(reason.contains("401")),
            other => panic!("expected Failed, got {:?}", other),
        }
    }

    // ── fraction ─────────────────────────────────────────────────────────────

    #[test]
    fn fraction_zero_total_returns_zero() {
        let t = PullProgressTracker::new();
        let id = t.start("a");
        let s = t.state(id).unwrap();
        assert_eq!(s.fraction(), 0.0);
    }

    #[test]
    fn fraction_partial_progress() {
        let t = PullProgressTracker::new();
        let id = t.start("a");
        t.manifest_fetched(id, 1, 200);
        t.layer_progress(id, "d", 50);
        let s = t.state(id).unwrap();
        assert!((s.fraction() - 0.25).abs() < 1e-6);
    }

    // ── list ─────────────────────────────────────────────────────────────────

    #[test]
    fn list_returns_all_active_pulls() {
        let t = PullProgressTracker::new();
        t.start("a");
        t.start("b");
        t.start("c");
        let all = t.list();
        assert_eq!(all.len(), 3);
    }

    // ── unknown id behaviour ────────────────────────────────────────────────

    #[test]
    fn state_unknown_returns_none() {
        let t = PullProgressTracker::new();
        assert!(t.state(Uuid::new_v4()).is_none());
    }

    #[test]
    fn events_unknown_returns_empty() {
        let t = PullProgressTracker::new();
        assert!(t.events(Uuid::new_v4()).is_empty());
    }

    #[test]
    fn manifest_fetched_unknown_id_is_silent() {
        let t = PullProgressTracker::new();
        t.manifest_fetched(Uuid::new_v4(), 1, 100);
        // No panic, but nothing queryable.
    }
}

//! Request recorder/replayer (Gravitee Debug mode).
//!
//! Captures requests/responses for debugging. Supports replay with modified upstream.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, RwLock};
use uuid::Uuid;

/// Decision log entry during request processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEntry {
    pub phase: String,
    pub plugin: String,
    pub action: String,
    pub payload: serde_json::Value,
}

/// Captured request for debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub id: Uuid,
    pub api_id: String,
    pub captured_at: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body_preview: String,
    pub response_status: Option<u16>,
    pub response_headers: HashMap<String, String>,
    pub response_body_preview: String,
    pub upstream_url: Option<String>,
    pub decision_log: Vec<DecisionEntry>,
}

/// Debug store and recorder.
pub struct DebugStore {
    enabled: AtomicBool,
    recordings: RwLock<Vec<RecordedRequest>>,
    max_recordings: usize,
}

impl DebugStore {
    /// Create a new debug store (disabled by default).
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            recordings: RwLock::new(Vec::with_capacity(1000)),
            max_recordings: 1000,
        })
    }

    /// Create with custom max recordings.
    pub fn with_capacity(max_recordings: usize) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            recordings: RwLock::new(Vec::with_capacity(max_recordings)),
            max_recordings,
        })
    }

    /// Toggle debug mode on/off.
    pub fn toggle(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Check if debug mode is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Capture a request (only if enabled).
    pub fn capture(&self, req: RecordedRequest) {
        if !self.is_enabled() {
            return;
        }

        let mut recordings = self.recordings.write().unwrap();
        recordings.push(req);

        // Enforce max recordings: drop oldest
        if recordings.len() > self.max_recordings {
            recordings.remove(0);
        }
    }

    /// List recordings, optionally filtered by API ID.
    pub fn list(&self, api_id_filter: Option<&str>) -> Vec<RecordedRequest> {
        let recordings = self.recordings.read().unwrap();
        recordings
            .iter()
            .filter(|r| api_id_filter.is_none() || api_id_filter == Some(&r.api_id))
            .cloned()
            .collect()
    }

    /// Get a specific recording by ID.
    pub fn get(&self, id: Uuid) -> Option<RecordedRequest> {
        let recordings = self.recordings.read().unwrap();
        recordings.iter().find(|r| r.id == id).cloned()
    }

    /// Replay a recording with an optional new upstream URL.
    /// Returns a replay plan (actual HTTP call is delegated to the caller).
    pub fn replay(&self, id: Uuid, new_upstream: Option<String>) -> Option<ReplayPlan> {
        let req = self.get(id)?;
        Some(ReplayPlan {
            original_id: id,
            method: req.method,
            path: req.path,
            query: req.query,
            headers: req.headers,
            body_preview: req.body_preview,
            upstream_url: new_upstream.unwrap_or(req.upstream_url?),
        })
    }

    /// Clear all recordings.
    pub fn clear(&self) {
        self.recordings.write().unwrap().clear();
    }
}

/// Plan for replaying a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayPlan {
    pub original_id: Uuid,
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body_preview: String,
    pub upstream_url: String,
}

impl Default for DebugStore {
    fn default() -> Self {
        DebugStore {
            enabled: AtomicBool::new(false),
            recordings: RwLock::new(Vec::with_capacity(1000)),
            max_recordings: 1000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_toggle_and_capture() {
        let store = DebugStore::new();
        assert!(!store.is_enabled());

        // Capture when disabled should be no-op
        store.capture(RecordedRequest {
            id: Uuid::new_v4(),
            api_id: "api1".to_string(),
            captured_at: Utc::now(),
            method: "GET".to_string(),
            path: "/test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body_preview: String::new(),
            response_status: Some(200),
            response_headers: HashMap::new(),
            response_body_preview: String::new(),
            upstream_url: Some("http://localhost:3000".to_string()),
            decision_log: vec![],
        });

        assert_eq!(store.list(None).len(), 0);

        // Enable and capture
        store.toggle(true);
        let req_id = Uuid::new_v4();
        store.capture(RecordedRequest {
            id: req_id,
            api_id: "api1".to_string(),
            captured_at: Utc::now(),
            method: "GET".to_string(),
            path: "/test".to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body_preview: String::new(),
            response_status: Some(200),
            response_headers: HashMap::new(),
            response_body_preview: String::new(),
            upstream_url: Some("http://localhost:3000".to_string()),
            decision_log: vec![],
        });

        assert_eq!(store.list(None).len(), 1);
        assert!(store.get(req_id).is_some());
    }

    #[test]
    fn test_debug_filter_by_api() {
        let store = DebugStore::new();
        store.toggle(true);

        for api_id in &["api1", "api2", "api1"] {
            store.capture(RecordedRequest {
                id: Uuid::new_v4(),
                api_id: api_id.to_string(),
                captured_at: Utc::now(),
                method: "GET".to_string(),
                path: "/test".to_string(),
                query: HashMap::new(),
                headers: HashMap::new(),
                body_preview: String::new(),
                response_status: Some(200),
                response_headers: HashMap::new(),
                response_body_preview: String::new(),
                upstream_url: Some("http://localhost:3000".to_string()),
                decision_log: vec![],
            });
        }

        let api1_reqs = store.list(Some("api1"));
        assert_eq!(api1_reqs.len(), 2);
        let api2_reqs = store.list(Some("api2"));
        assert_eq!(api2_reqs.len(), 1);
    }

    #[test]
    fn test_debug_replay_plan() {
        let store = DebugStore::new();
        store.toggle(true);

        let req_id = Uuid::new_v4();
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token123".to_string());

        store.capture(RecordedRequest {
            id: req_id,
            api_id: "api1".to_string(),
            captured_at: Utc::now(),
            method: "POST".to_string(),
            path: "/users".to_string(),
            query: HashMap::new(),
            headers: headers.clone(),
            body_preview: "{\"name\": \"Alice\"}".to_string(),
            response_status: Some(201),
            response_headers: HashMap::new(),
            response_body_preview: "{\"id\": 1}".to_string(),
            upstream_url: Some("http://old-backend:3000".to_string()),
            decision_log: vec![],
        });

        let plan = store.replay(req_id, Some("http://new-backend:3000".to_string()));
        assert!(plan.is_some());
        let p = plan.unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.path, "/users");
        assert_eq!(p.upstream_url, "http://new-backend:3000");
        assert_eq!(p.headers.get("Authorization").unwrap(), "Bearer token123");
    }
}

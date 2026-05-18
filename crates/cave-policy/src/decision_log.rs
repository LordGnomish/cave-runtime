// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decision logging for OPA policy evaluation.
//!
//! Logs every policy decision with input, result, metrics, and provenance.
//! Used for compliance, audit, debugging.

use crate::models::DecisionLogEntry;
use chrono::Utc;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// In-memory + DB decision log.
pub struct DecisionLog {
    /// Recent decisions (bounded ring buffer for in-memory access).
    recent: Arc<Mutex<Vec<DecisionLogEntry>>>,
    capacity: usize,
    enabled: bool,
}

impl DecisionLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            recent: Arc::new(Mutex::new(Vec::with_capacity(capacity))),
            capacity,
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            recent: Arc::new(Mutex::new(Vec::new())),
            capacity: 0,
            enabled: false,
        }
    }

    /// Log a policy decision.
    pub fn record(
        &self,
        path: &str,
        input: Option<&serde_json::Value>,
        result: Option<&serde_json::Value>,
        error: Option<&str>,
        requested_by: &str,
    ) -> DecisionLogEntry {
        let entry = DecisionLogEntry {
            decision_id: Uuid::new_v4().to_string(),
            path: path.to_string(),
            input: input.cloned(),
            result: result.cloned(),
            error: error.map(String::from),
            requested_by: requested_by.to_string(),
            timestamp: Utc::now(),
            metrics: None,
            bundle_name: None,
            revision: None,
        };

        if self.enabled {
            if let Ok(mut guard) = self.recent.lock() {
                if guard.len() >= self.capacity {
                    guard.remove(0);
                }
                guard.push(entry.clone());
            }

            tracing::debug!(
                target: "cave_policy.decision",
                decision_id = entry.decision_id,
                path = path,
                allowed = result.and_then(|v| v.as_bool()).unwrap_or(false),
                "policy decision recorded"
            );
        }

        entry
    }

    /// Get recent decisions (most recent first).
    pub fn recent_decisions(&self, limit: usize) -> Vec<DecisionLogEntry> {
        if let Ok(guard) = self.recent.lock() {
            guard.iter().rev().take(limit).cloned().collect()
        } else {
            vec![]
        }
    }

    /// Check if logging is enabled.
    pub fn is_enabled(&self) -> bool { self.enabled }
}

impl Default for DecisionLog {
    fn default() -> Self { Self::new(1000) }
}

/// Mask sensitive fields in a decision log entry before storage.
pub fn mask_sensitive(entry: &mut DecisionLogEntry, mask_fields: &[&str]) {
    if let Some(input) = &mut entry.input {
        mask_json(input, mask_fields);
    }
    if let Some(result) = &mut entry.result {
        mask_json(result, mask_fields);
    }
}

fn mask_json(v: &mut serde_json::Value, fields: &[&str]) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, val) in m.iter_mut() {
                if fields.iter().any(|f| k.to_lowercase().contains(f)) {
                    *val = serde_json::json!("***MASKED***");
                } else {
                    mask_json(val, fields);
                }
            }
        }
        serde_json::Value::Array(a) => {
            for item in a { mask_json(item, fields); }
        }
        _ => {}
    }
}

/// Metrics collected during policy evaluation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct EvalMetrics {
    pub timer_rego_query_parse_ns: u64,
    pub timer_rego_query_compile_ns: u64,
    pub timer_rego_query_eval_ns: u64,
    pub timer_server_handler_ns: u64,
    pub counter_server_query_cache_hit: u64,
}

impl EvalMetrics {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "timer_rego_query_parse_ns": self.timer_rego_query_parse_ns,
            "timer_rego_query_compile_ns": self.timer_rego_query_compile_ns,
            "timer_rego_query_eval_ns": self.timer_rego_query_eval_ns,
            "timer_server_handler_ns": self.timer_server_handler_ns,
            "counter_server_query_cache_hit": self.counter_server_query_cache_hit,
        })
    }
}

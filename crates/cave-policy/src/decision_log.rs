//! Decision logging — record every policy evaluation with input, result, and metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// A single logged policy decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub policy_id: String,
    pub input: serde_json::Value,
    pub result: serde_json::Value,
    pub decision_id: String,
    pub path: String,
    pub metrics: DecisionMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionMetrics {
    /// Time to evaluate in microseconds.
    pub timer_rego_query_eval_us: u64,
}

/// Thread-safe, bounded in-memory decision log.
#[derive(Debug, Clone)]
pub struct DecisionLog {
    inner: Arc<Mutex<LogInner>>,
}

#[derive(Debug)]
struct LogInner {
    entries: VecDeque<DecisionEntry>,
    max_entries: usize,
}

impl DecisionLog {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogInner {
                entries: VecDeque::new(),
                max_entries,
            })),
        }
    }

    /// Append a new decision entry, evicting the oldest if at capacity.
    pub fn record(
        &self,
        policy_id: impl Into<String>,
        path: impl Into<String>,
        input: serde_json::Value,
        result: serde_json::Value,
        eval_us: u64,
    ) {
        let entry = DecisionEntry {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            policy_id: policy_id.into(),
            input,
            result,
            decision_id: Uuid::new_v4().to_string(),
            path: path.into(),
            metrics: DecisionMetrics { timer_rego_query_eval_us: eval_us },
        };
        let mut lock = self.inner.lock().expect("decision log lock poisoned");
        if lock.entries.len() >= lock.max_entries {
            lock.entries.pop_front();
        }
        lock.entries.push_back(entry);
    }

    /// Return all entries (oldest first), up to `limit`.
    pub fn entries(&self, limit: usize) -> Vec<DecisionEntry> {
        let lock = self.inner.lock().expect("decision log lock poisoned");
        lock.entries.iter().rev().take(limit).cloned().collect::<Vec<_>>()
            .into_iter().rev().collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("lock poisoned").entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for DecisionLog {
    fn default() -> Self {
        Self::new(10_000)
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory cache store with TTL, pattern matching, atomic ops, pipeline, and pub/sub.

use crate::models::{CacheEntry, CacheStats, EvictionPolicy, PubSubMessage};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

const MAX_CHANNEL_MESSAGES: usize = 100;

/// Per-channel pub/sub state.
pub struct ChannelState {
    pub message_count: u64,
    pub messages: VecDeque<PubSubMessage>,
}

/// The core in-memory cache, guarded externally by a Mutex.
pub struct CacheStore {
    pub entries: HashMap<String, CacheEntry>,
    pub stats: CacheStats,
    pub eviction_policy: EvictionPolicy,
    pub channels: HashMap<String, ChannelState>,
}

impl CacheStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            stats: CacheStats::default(),
            eviction_policy: EvictionPolicy::default(),
            channels: HashMap::new(),
        }
    }

    /// Retrieve a key. Evicts and returns `None` if expired.
    pub fn get(&mut self, key: &str) -> Option<CacheEntry> {
        // Expire check (borrow ends here — value is Copy/owned)
        let expired = self
            .entries
            .get(key)
            .and_then(|e| e.expires_at)
            .map(|exp| Utc::now() > exp)
            .unwrap_or(false);

        if expired {
            self.entries.remove(key);
            self.stats.evictions += 1;
            self.stats.total_keys = self.entries.len();
            self.stats.misses += 1;
            return None;
        }

        if let Some(entry) = self.entries.get_mut(key) {
            entry.access_count += 1;
            entry.last_accessed = Utc::now();
            self.stats.hits += 1;
            Some(entry.clone())
        } else {
            self.stats.misses += 1;
            None
        }
    }

    /// Insert or replace a key.
    pub fn set(
        &mut self,
        key: String,
        value: serde_json::Value,
        ttl: Option<u64>,
        tags: Vec<String>,
    ) {
        let now = Utc::now();
        let expires_at = ttl.map(|secs| {
            now + chrono::Duration::try_seconds(secs as i64).unwrap_or_default()
        });
        self.entries.insert(
            key.clone(),
            CacheEntry {
                key,
                value,
                ttl,
                tags,
                created_at: now,
                expires_at,
                access_count: 0,
                last_accessed: now,
            },
        );
        self.stats.total_keys = self.entries.len();
    }

    /// Remove a key. Returns `true` if it existed.
    pub fn delete(&mut self, key: &str) -> bool {
        let existed = self.entries.remove(key).is_some();
        self.stats.total_keys = self.entries.len();
        existed
    }

    /// Update the TTL on an existing key. Returns `false` if missing.
    pub fn expire(&mut self, key: &str, ttl_secs: u64) -> bool {
        if let Some(entry) = self.entries.get_mut(key) {
            let exp = Utc::now()
                + chrono::Duration::try_seconds(ttl_secs as i64).unwrap_or_default();
            entry.ttl = Some(ttl_secs);
            entry.expires_at = Some(exp);
            true
        } else {
            false
        }
    }

    /// Atomically add `by` to a numeric key (creates the key at 0 if absent).
    pub fn incr(&mut self, key: &str, by: i64) -> Result<i64, String> {
        let now = Utc::now();
        let entry = self.entries.entry(key.to_string()).or_insert_with(|| CacheEntry {
            key: key.to_string(),
            value: serde_json::json!(0i64),
            ttl: None,
            tags: vec![],
            created_at: now,
            expires_at: None,
            access_count: 0,
            last_accessed: now,
        });

        // Use `entry` fully before touching self.entries again.
        let cur = match &entry.value {
            serde_json::Value::Number(n) => {
                n.as_i64().ok_or_else(|| "value is not an integer".to_string())?
            }
            _ => return Err("value is not a number".to_string()),
        };
        let next = cur + by;
        entry.value = serde_json::json!(next);
        // `entry` borrow ends here — safe to touch self.entries.
        self.stats.total_keys = self.entries.len();
        Ok(next)
    }

    /// Return all keys matching `pattern` (supports `*` and `?` glob wildcards).
    pub fn keys_matching(&self, pattern: &str) -> Vec<String> {
        self.entries
            .keys()
            .filter(|k| glob_match(pattern, k))
            .cloned()
            .collect()
    }

    /// Execute a batch of operations, returning results in order.
    pub fn pipeline(&mut self, ops: Vec<PipelineOp>) -> Vec<PipelineResult> {
        ops.into_iter().map(|op| self.exec_op(op)).collect()
    }

    fn exec_op(&mut self, op: PipelineOp) -> PipelineResult {
        match op.op.as_str() {
            "get" => match self.get(&op.key) {
                Some(e) => PipelineResult::Value(e.value),
                None => PipelineResult::Nil,
            },
            "set" => {
                let val = op.value.unwrap_or(serde_json::Value::Null);
                self.set(op.key, val, op.ttl, op.tags.unwrap_or_default());
                PipelineResult::Ok
            }
            "delete" => PipelineResult::Integer(if self.delete(&op.key) { 1 } else { 0 }),
            "incr" => match self.incr(&op.key, op.by.unwrap_or(1)) {
                Ok(n) => PipelineResult::Integer(n),
                Err(e) => PipelineResult::Error(e),
            },
            "decr" => match self.incr(&op.key, -op.by.unwrap_or(1)) {
                Ok(n) => PipelineResult::Integer(n),
                Err(e) => PipelineResult::Error(e),
            },
            "expire" => {
                let ttl = op.ttl.unwrap_or(0);
                PipelineResult::Integer(if self.expire(&op.key, ttl) { 1 } else { 0 })
            }
            other => PipelineResult::Error(format!("unknown op: {other}")),
        }
    }

    /// Publish `payload` to `channel`. Returns the channel's total message count.
    pub fn publish(&mut self, channel: String, payload: serde_json::Value) -> u64 {
        let msg = PubSubMessage {
            message_id: Uuid::new_v4(),
            channel: channel.clone(),
            payload,
            timestamp: Utc::now(),
        };
        // Scope the mutable borrow so we can call self.channels.len() afterwards.
        let msg_count = {
            let ch = self.channels.entry(channel).or_insert_with(|| ChannelState {
                message_count: 0,
                messages: VecDeque::new(),
            });
            ch.message_count += 1;
            ch.messages.push_back(msg);
            if ch.messages.len() > MAX_CHANNEL_MESSAGES {
                ch.messages.pop_front();
            }
            ch.message_count
        };
        self.stats.pubsub_messages_total += 1;
        self.stats.pubsub_channels = self.channels.len();
        msg_count
    }
}

// ── Pipeline types ────────────────────────────────────────────────────────────

/// A single operation within a pipeline request.
#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineOp {
    /// One of: `get`, `set`, `delete`, `incr`, `decr`, `expire`.
    pub op: String,
    pub key: String,
    pub value: Option<serde_json::Value>,
    pub ttl: Option<u64>,
    pub tags: Option<Vec<String>>,
    /// Amount to increment/decrement (default 1).
    pub by: Option<i64>,
}

/// Result of a single pipeline operation.
#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum PipelineResult {
    Ok,
    Nil,
    Integer(i64),
    Value(serde_json::Value),
    Error(String),
}

// ── Glob matching ─────────────────────────────────────────────────────────────

fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_inner(&p, &t)
}

fn glob_inner(p: &[char], t: &[char]) -> bool {
    match (p.first(), t.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(&'*'), _) => glob_inner(&p[1..], t) || (!t.is_empty() && glob_inner(p, &t[1..])),
        (Some(&'?'), Some(_)) => glob_inner(&p[1..], &t[1..]),
        (Some(pc), Some(tc)) if pc == tc => glob_inner(&p[1..], &t[1..]),
        _ => false,
    }
}

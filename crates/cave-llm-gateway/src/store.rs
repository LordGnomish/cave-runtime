// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for CAVE LLM Gateway.

use crate::models::{
    BudgetEntityType, BudgetLimit, CacheEntry, ChatMessage, RateLimit, RateLimitEntityType,
    RequestLog, SpendRecord,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Thread-safe in-memory store for the gateway.
#[derive(Default)]
pub struct LlmGatewayStore {
    spend_records: Arc<Mutex<Vec<SpendRecord>>>,
    budget_limits: Arc<Mutex<Vec<BudgetLimit>>>,
    rate_limits: Arc<Mutex<HashMap<String, RateLimit>>>,
    request_logs: Arc<Mutex<Vec<RequestLog>>>,
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

impl LlmGatewayStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    // ── Spend tracking ────────────────────────────────────────────────────────

    /// Record a spend entry.
    pub fn record_spend(&self, record: SpendRecord) {
        self.spend_records.lock().unwrap().push(record);
    }

    /// Sum spend for a specific user after `period_start`.
    #[allow(dead_code)]
    pub fn get_spend_by_user(&self, user_id: &str, period_start: DateTime<Utc>) -> f64 {
        self.spend_records
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                r.user_id.as_deref() == Some(user_id) && r.created_at >= period_start
            })
            .map(|r| r.cost_usd)
            .sum()
    }

    /// Sum spend for a specific team after `period_start`.
    #[allow(dead_code)]
    pub fn get_spend_by_team(&self, team_id: &str, period_start: DateTime<Utc>) -> f64 {
        self.spend_records
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                r.team_id.as_deref() == Some(team_id) && r.created_at >= period_start
            })
            .map(|r| r.cost_usd)
            .sum()
    }

    /// Returns `true` if the entity is within their budget.
    pub fn check_budget(&self, entity_type: &BudgetEntityType, entity_id: &str) -> bool {
        let limits = self.budget_limits.lock().unwrap();
        if let Some(budget) = limits
            .iter()
            .find(|b| &b.entity_type == entity_type && b.entity_id == entity_id)
        {
            return budget.current_spend < budget.limit_usd;
        }
        // No budget configured means unlimited.
        true
    }

    /// Return the most recent `limit` spend records.
    #[allow(dead_code)]
    pub fn list_spend_records(&self, limit: usize) -> Vec<SpendRecord> {
        let records = self.spend_records.lock().unwrap();
        records.iter().rev().take(limit).cloned().collect()
    }

    /// Add a budget limit.
    pub fn add_budget_limit(&self, budget: BudgetLimit) {
        self.budget_limits.lock().unwrap().push(budget);
    }

    /// Return all budget limits.
    #[allow(dead_code)]
    pub fn list_budget_limits(&self) -> Vec<BudgetLimit> {
        self.budget_limits.lock().unwrap().clone()
    }

    // ── Rate limiting ─────────────────────────────────────────────────────────

    /// Returns `true` if the user is within their rate limit window.
    ///
    /// A sliding 1-minute window is maintained per user. The default limits are
    /// 60 requests/minute and 100 000 tokens/minute.
    pub fn check_rate_limit(&self, user_id: &str, tokens: u32) -> bool {
        let mut limits = self.rate_limits.lock().unwrap();
        let now = Utc::now();

        if let Some(rl) = limits.get_mut(user_id) {
            // Reset window if older than 1 minute.
            if now - rl.window_start > Duration::minutes(1) {
                rl.current_requests = 0;
                rl.current_tokens = 0;
                rl.window_start = now;
            }
            rl.current_requests < rl.requests_per_minute
                && rl.current_tokens + tokens <= rl.tokens_per_minute
        } else {
            // First request — create entry.
            limits.insert(
                user_id.to_string(),
                RateLimit {
                    entity_type: RateLimitEntityType::User,
                    entity_id: user_id.to_string(),
                    requests_per_minute: 60,
                    tokens_per_minute: 100_000,
                    current_requests: 0,
                    current_tokens: 0,
                    window_start: now,
                },
            );
            true
        }
    }

    /// Increment the request and token counters for a user.
    #[allow(dead_code)]
    pub fn increment_rate_counter(&self, user_id: &str, tokens: u32) {
        let mut limits = self.rate_limits.lock().unwrap();
        let now = Utc::now();
        let entry = limits.entry(user_id.to_string()).or_insert_with(|| RateLimit {
            entity_type: RateLimitEntityType::User,
            entity_id: user_id.to_string(),
            requests_per_minute: 60,
            tokens_per_minute: 100_000,
            current_requests: 0,
            current_tokens: 0,
            window_start: now,
        });

        // Reset if window expired.
        if now - entry.window_start > Duration::minutes(1) {
            entry.current_requests = 0;
            entry.current_tokens = 0;
            entry.window_start = now;
        }

        entry.current_requests += 1;
        entry.current_tokens += tokens;
    }

    // ── Caching ───────────────────────────────────────────────────────────────

    /// Generate a deterministic cache key from model name + message content.
    pub fn cache_key(model: &str, messages: &[ChatMessage]) -> String {
        let content = serde_json::to_string(messages).unwrap_or_default();
        let snippet: String = content.chars().take(200).collect();
        format!("{model}::{snippet}")
    }

    /// Retrieve a cached response, incrementing the hit counter.
    pub fn get_cached(&self, key: &str) -> Option<Value> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(key) {
            entry.hit_count += 1;
            return Some(entry.response.clone());
        }
        None
    }

    /// Store a response in the cache.
    pub fn set_cache(&self, key: String, response: Value, model: String) {
        let entry = CacheEntry {
            key: key.clone(),
            response,
            created_at: Utc::now(),
            hit_count: 0,
            model,
        };
        self.cache.lock().unwrap().insert(key, entry);
    }

    // ── Request logging ───────────────────────────────────────────────────────

    /// Append a request log entry.
    pub fn log_request(&self, log: RequestLog) {
        self.request_logs.lock().unwrap().push(log);
    }

    /// Return the most recent `limit` request log entries.
    pub fn list_logs(&self, limit: usize) -> Vec<RequestLog> {
        let logs = self.request_logs.lock().unwrap();
        logs.iter().rev().take(limit).cloned().collect()
    }

    // ── Analytics ─────────────────────────────────────────────────────────────

    /// Return a summary of gateway activity.
    pub fn analytics_summary(&self) -> Value {
        let logs = self.request_logs.lock().unwrap();
        let total_requests = logs.len();
        let cached_requests = logs.iter().filter(|l| l.was_cached).count();
        let cache_hit_rate = if total_requests > 0 {
            cached_requests as f64 / total_requests as f64
        } else {
            0.0
        };
        drop(logs);

        let records = self.spend_records.lock().unwrap();
        let total_tokens: u32 = records.iter().map(|r| r.total_tokens).sum();
        let total_cost_usd: f64 = records.iter().map(|r| r.cost_usd).sum();

        // Count usage per model.
        let mut model_counts: HashMap<String, u32> = HashMap::new();
        for r in records.iter() {
            *model_counts.entry(r.model.clone()).or_insert(0) += 1;
        }
        drop(records);

        let mut top_models: Vec<(String, u32)> = model_counts.into_iter().collect();
        top_models.sort_by(|a, b| b.1.cmp(&a.1));
        let top_models: Vec<Value> = top_models
            .into_iter()
            .take(5)
            .map(|(m, c)| serde_json::json!({ "model": m, "requests": c }))
            .collect();

        serde_json::json!({
            "total_requests": total_requests,
            "total_tokens": total_tokens,
            "total_cost_usd": total_cost_usd,
            "cache_hit_rate": cache_hit_rate,
            "top_models": top_models,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BudgetPeriod, Provider};
    use uuid::Uuid;

    fn sample_spend(user_id: &str, cost: f64) -> SpendRecord {
        SpendRecord {
            id: Uuid::new_v4(),
            user_id: Some(user_id.to_string()),
            team_id: None,
            project_id: None,
            provider: Provider::OpenAi,
            model: "gpt-4o".to_string(),
            prompt_tokens: 10,
            completion_tokens: 8,
            total_tokens: 18,
            cost_usd: cost,
            request_id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_record_and_retrieve_spend() {
        let store = LlmGatewayStore::new();
        store.record_spend(sample_spend("alice", 0.01));
        store.record_spend(sample_spend("alice", 0.02));
        let records = store.list_spend_records(10);
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn test_get_spend_by_user() {
        let store = LlmGatewayStore::new();
        store.record_spend(sample_spend("alice", 0.10));
        store.record_spend(sample_spend("bob", 0.05));
        store.record_spend(sample_spend("alice", 0.20));
        let period_start = Utc::now() - Duration::hours(1);
        let alice_spend = store.get_spend_by_user("alice", period_start);
        assert!((alice_spend - 0.30).abs() < 1e-9);
    }

    #[test]
    fn test_budget_within_limit() {
        let store = LlmGatewayStore::new();
        store.add_budget_limit(BudgetLimit {
            id: Uuid::new_v4(),
            entity_type: BudgetEntityType::User,
            entity_id: "alice".to_string(),
            limit_usd: 10.0,
            period: BudgetPeriod::Monthly,
            current_spend: 5.0,
            reset_at: Utc::now() + Duration::days(30),
        });
        assert!(store.check_budget(&BudgetEntityType::User, "alice"));
    }

    #[test]
    fn test_budget_over_limit() {
        let store = LlmGatewayStore::new();
        store.add_budget_limit(BudgetLimit {
            id: Uuid::new_v4(),
            entity_type: BudgetEntityType::User,
            entity_id: "bob".to_string(),
            limit_usd: 5.0,
            period: BudgetPeriod::Monthly,
            current_spend: 5.50, // over limit
            reset_at: Utc::now() + Duration::days(30),
        });
        assert!(!store.check_budget(&BudgetEntityType::User, "bob"));
    }

    #[test]
    fn test_budget_no_limit_configured_allows() {
        let store = LlmGatewayStore::new();
        // No budget configured → unlimited.
        assert!(store.check_budget(&BudgetEntityType::User, "unknown_user"));
    }

    #[test]
    fn test_rate_limit_first_request_allowed() {
        let store = LlmGatewayStore::new();
        assert!(store.check_rate_limit("user1", 100));
    }

    #[test]
    fn test_rate_limit_token_exhaustion() {
        let store = LlmGatewayStore::new();
        // Increment to just below token limit.
        store.increment_rate_counter("heavy_user", 99_999);
        // Now requesting 2 more tokens should be denied (100_001 > 100_000).
        assert!(!store.check_rate_limit("heavy_user", 2));
    }

    #[test]
    fn test_cache_miss() {
        let store = LlmGatewayStore::new();
        let key = LlmGatewayStore::cache_key("gpt-4o", &[]);
        assert!(store.get_cached(&key).is_none());
    }

    #[test]
    fn test_cache_set_and_hit() {
        let store = LlmGatewayStore::new();
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }];
        let key = LlmGatewayStore::cache_key("gpt-4o", &messages);
        let value = serde_json::json!({ "answer": 42 });
        store.set_cache(key.clone(), value.clone(), "gpt-4o".to_string());
        let cached = store.get_cached(&key);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap(), value);
    }

    #[test]
    fn test_cache_key_same_messages_same_key() {
        let msgs = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello, world!".to_string(),
        }];
        let k1 = LlmGatewayStore::cache_key("gpt-4o", &msgs);
        let k2 = LlmGatewayStore::cache_key("gpt-4o", &msgs);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_cache_key_different_messages_different_key() {
        let msgs_a = vec![ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        }];
        let msgs_b = vec![ChatMessage {
            role: "user".to_string(),
            content: "Goodbye".to_string(),
        }];
        let k1 = LlmGatewayStore::cache_key("gpt-4o", &msgs_a);
        let k2 = LlmGatewayStore::cache_key("gpt-4o", &msgs_b);
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_request_logging() {
        let store = LlmGatewayStore::new();
        let log = RequestLog {
            id: Uuid::new_v4(),
            request_id: "req-1".to_string(),
            user_id: Some("alice".to_string()),
            provider: Provider::OpenAi,
            model: "gpt-4o".to_string(),
            request_type: "chat".to_string(),
            prompt_tokens: Some(10),
            completion_tokens: Some(8),
            latency_ms: 250,
            status_code: 200,
            error: None,
            was_cached: false,
            created_at: Utc::now(),
        };
        store.log_request(log);
        let logs = store.list_logs(10);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].request_id, "req-1");
    }

    #[test]
    fn test_analytics_summary() {
        let store = LlmGatewayStore::new();
        store.record_spend(sample_spend("alice", 0.05));
        store.log_request(RequestLog {
            id: Uuid::new_v4(),
            request_id: "r1".to_string(),
            user_id: Some("alice".to_string()),
            provider: Provider::OpenAi,
            model: "gpt-4o".to_string(),
            request_type: "chat".to_string(),
            prompt_tokens: Some(10),
            completion_tokens: Some(8),
            latency_ms: 100,
            status_code: 200,
            error: None,
            was_cached: false,
            created_at: Utc::now(),
        });
        let summary = store.analytics_summary();
        assert_eq!(summary["total_requests"], 1);
        assert_eq!(summary["total_tokens"], 18);
        assert!((summary["total_cost_usd"].as_f64().unwrap() - 0.05).abs() < 1e-9);
    }
}

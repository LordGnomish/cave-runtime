// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Token counting and cost tracking.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

// ── Pricing catalog ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub model: String,
    /// Cost per 1M input tokens in USD
    pub input_per_million: f64,
    /// Cost per 1M output tokens in USD
    pub output_per_million: f64,
}

impl ModelPricing {
    pub fn cost(&self, input_tokens: u32, output_tokens: u32) -> f64 {
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_per_million;
        input_cost + output_cost
    }
}

pub fn default_pricing() -> Vec<ModelPricing> {
    vec![
        // OpenAI
        ModelPricing {
            model: "gpt-4o".into(),
            input_per_million: 2.50,
            output_per_million: 10.00,
        },
        ModelPricing {
            model: "gpt-4o-mini".into(),
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
        ModelPricing {
            model: "gpt-4-turbo".into(),
            input_per_million: 10.00,
            output_per_million: 30.00,
        },
        ModelPricing {
            model: "gpt-4".into(),
            input_per_million: 30.00,
            output_per_million: 60.00,
        },
        ModelPricing {
            model: "gpt-3.5-turbo".into(),
            input_per_million: 0.50,
            output_per_million: 1.50,
        },
        // Anthropic
        ModelPricing {
            model: "claude-opus-4-6".into(),
            input_per_million: 15.00,
            output_per_million: 75.00,
        },
        ModelPricing {
            model: "claude-sonnet-4-6".into(),
            input_per_million: 3.00,
            output_per_million: 15.00,
        },
        ModelPricing {
            model: "claude-haiku-4-5-20251001".into(),
            input_per_million: 0.80,
            output_per_million: 4.00,
        },
        ModelPricing {
            model: "claude-3-5-sonnet-20241022".into(),
            input_per_million: 3.00,
            output_per_million: 15.00,
        },
        ModelPricing {
            model: "claude-3-5-haiku-20241022".into(),
            input_per_million: 0.80,
            output_per_million: 4.00,
        },
        // Groq (OpenAI-compat SaaS)
        ModelPricing {
            model: "llama-3.3-70b-versatile".into(),
            input_per_million: 0.59,
            output_per_million: 0.79,
        },
        ModelPricing {
            model: "llama-3.1-8b-instant".into(),
            input_per_million: 0.05,
            output_per_million: 0.08,
        },
        // DeepSeek (OpenAI-compat SaaS)
        ModelPricing {
            model: "deepseek-v4-flash".into(),
            input_per_million: 0.27,
            output_per_million: 1.10,
        },
        ModelPricing {
            model: "deepseek-chat".into(),
            input_per_million: 0.27,
            output_per_million: 1.10,
        },
        // Together AI (OpenAI-compat SaaS)
        ModelPricing {
            model: "meta-llama/Llama-3.3-70B-Instruct-Turbo".into(),
            input_per_million: 0.88,
            output_per_million: 0.88,
        },
        // Local / mock — free
        ModelPricing {
            model: "llama3".into(),
            input_per_million: 0.0,
            output_per_million: 0.0,
        },
        ModelPricing {
            model: "mock-model".into(),
            input_per_million: 0.0,
            output_per_million: 0.0,
        },
    ]
}

// ── Usage record ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub consumer: String,
    pub model: String,
    pub provider: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub cost_usd: f64,
    pub timestamp_ms: i64,
}

// ── Aggregate stats ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageStats {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
}

impl UsageStats {
    fn add(&mut self, record: &UsageRecord) {
        self.total_requests += 1;
        self.total_input_tokens += record.input_tokens as u64;
        self.total_output_tokens += record.output_tokens as u64;
        self.total_tokens += record.total_tokens as u64;
        self.total_cost_usd += record.cost_usd;
    }
}

// ── Cost tracker ──────────────────────────────────────────────────────────────

pub struct CostTracker {
    pricing: DashMap<String, ModelPricing>,
    /// Per-consumer aggregate stats
    consumer_stats: DashMap<String, UsageStats>,
    /// Per-model aggregate stats
    model_stats: DashMap<String, UsageStats>,
    /// Global stats
    global: std::sync::RwLock<UsageStats>,
}

impl CostTracker {
    pub fn new() -> Self {
        let tracker = Self {
            pricing: DashMap::new(),
            consumer_stats: DashMap::new(),
            model_stats: DashMap::new(),
            global: std::sync::RwLock::new(UsageStats::default()),
        };
        for p in default_pricing() {
            tracker.pricing.insert(p.model.clone(), p);
        }
        tracker
    }

    pub fn register_pricing(&self, pricing: ModelPricing) {
        self.pricing.insert(pricing.model.clone(), pricing);
    }

    pub fn cost_for(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        self.pricing
            .get(model)
            .map(|p| p.cost(input_tokens, output_tokens))
            .unwrap_or(0.0)
    }

    pub fn record(
        &self,
        consumer: &str,
        model: &str,
        provider: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> UsageRecord {
        let cost = self.cost_for(model, input_tokens, output_tokens);
        let record = UsageRecord {
            consumer: consumer.to_string(),
            model: model.to_string(),
            provider: provider.to_string(),
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            cost_usd: cost,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        };

        self.consumer_stats
            .entry(consumer.to_string())
            .or_default()
            .add(&record);
        self.model_stats
            .entry(model.to_string())
            .or_default()
            .add(&record);

        if let Ok(mut g) = self.global.write() {
            g.add(&record);
        }

        record
    }

    pub fn consumer_stats(&self, consumer: &str) -> UsageStats {
        self.consumer_stats
            .get(consumer)
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn model_stats(&self, model: &str) -> UsageStats {
        self.model_stats
            .get(model)
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn global_stats(&self) -> UsageStats {
        self.global.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn list_consumers(&self) -> Vec<String> {
        self.consumer_stats
            .iter()
            .map(|e| e.key().clone())
            .collect()
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ── Naive token counter ───────────────────────────────────────────────────────

/// Rough token estimate: ~4 chars per token.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as f64 / 4.0).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_calculation() {
        let pricing = ModelPricing {
            model: "test".into(),
            input_per_million: 2.0,
            output_per_million: 8.0,
        };
        // 1M input = $2, 1M output = $8
        let cost = pricing.cost(1_000_000, 1_000_000);
        assert!((cost - 10.0).abs() < 0.001);
    }

    #[test]
    fn zero_cost_for_local_model() {
        let tracker = CostTracker::new();
        let cost = tracker.cost_for("llama3", 50000, 10000);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn record_and_query_stats() {
        let tracker = CostTracker::new();
        tracker.record("alice", "gpt-4o-mini", "openai", 100, 50);
        tracker.record("alice", "gpt-4o-mini", "openai", 200, 100);
        let stats = tracker.consumer_stats("alice");
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.total_input_tokens, 300);
    }

    #[test]
    fn estimate_tokens_rough() {
        // "Hello world" ≈ 3 tokens, rough estimate is ok
        let t = estimate_tokens("Hello world");
        assert!(t >= 2 && t <= 5);
    }
}

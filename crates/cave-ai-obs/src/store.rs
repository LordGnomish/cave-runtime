// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{
    AiObsStats, CostStats, LatencyStats, LlmRequest, RequestStatus, TokenStats,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::RwLock;

const MAX_REQUESTS: usize = 10_000;

#[derive(Default)]
pub struct AiObsStore {
    requests: RwLock<Vec<LlmRequest>>,
}

impl AiObsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&self, req: LlmRequest) {
        let mut requests = self.requests.write().unwrap();
        if requests.len() >= MAX_REQUESTS {
            requests.remove(0);
        }
        requests.push(req);
    }

    pub fn get_by_id(&self, id: &uuid::Uuid) -> Option<LlmRequest> {
        self.requests
            .read()
            .unwrap()
            .iter()
            .find(|r| r.id == *id)
            .cloned()
    }

    pub fn filter_requests(
        &self,
        provider: Option<&str>,
        model: Option<&str>,
        user_id: Option<&str>,
        status: Option<&str>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Vec<LlmRequest> {
        let requests = self.requests.read().unwrap();
        let mut results: Vec<LlmRequest> = requests
            .iter()
            .filter(|r| {
                if let Some(p) = provider {
                    let p_str = format!("{:?}", r.provider).to_lowercase();
                    if !p_str.contains(&p.to_lowercase()) {
                        return false;
                    }
                }
                if let Some(m) = model {
                    if !r.model.to_lowercase().contains(&m.to_lowercase()) {
                        return false;
                    }
                }
                if let Some(u) = user_id {
                    if r.user_id.as_deref() != Some(u) {
                        return false;
                    }
                }
                if let Some(s) = status {
                    let s_str = format!("{:?}", r.status).to_lowercase();
                    if !s_str.contains(&s.to_lowercase()) {
                        return false;
                    }
                }
                if let Some(f) = from {
                    if r.created_at < f {
                        return false;
                    }
                }
                if let Some(t) = to {
                    if r.created_at > t {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        // newest first
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        results.truncate(limit);
        results
    }

    pub fn all_requests(&self) -> Vec<LlmRequest> {
        self.requests.read().unwrap().clone()
    }

    pub fn compute_stats(&self) -> AiObsStats {
        let requests = self.requests.read().unwrap();
        let count = requests.len() as u64;
        if count == 0 {
            return AiObsStats::default();
        }
        let success_count = requests
            .iter()
            .filter(|r| matches!(r.status, RequestStatus::Success))
            .count() as f64;
        let success_rate = success_count / count as f64;

        let total_prompt: u64 = requests.iter().map(|r| r.prompt_tokens as u64).sum();
        let total_completion: u64 = requests.iter().map(|r| r.completion_tokens as u64).sum();
        let token_stats = TokenStats {
            total_prompt,
            total_completion,
            total: total_prompt + total_completion,
            avg_prompt_per_request: total_prompt as f64 / count as f64,
            avg_completion_per_request: total_completion as f64 / count as f64,
        };

        let cost_stats = self.compute_cost_stats_inner(&requests);
        let latency_stats = self.compute_latency_stats_inner(&requests);

        // error rate by model
        let mut model_total: HashMap<String, u64> = HashMap::new();
        let mut model_errors: HashMap<String, u64> = HashMap::new();
        for r in requests.iter() {
            *model_total.entry(r.model.clone()).or_default() += 1;
            if !matches!(r.status, RequestStatus::Success) {
                *model_errors.entry(r.model.clone()).or_default() += 1;
            }
        }
        let error_rate_by_model = model_total
            .iter()
            .map(|(m, total)| {
                let errors = model_errors.get(m).copied().unwrap_or(0);
                (m.clone(), errors as f64 / *total as f64)
            })
            .collect();

        AiObsStats {
            request_count: count,
            success_rate,
            token_stats,
            cost_stats,
            latency_stats,
            error_rate_by_model,
        }
    }

    pub fn compute_cost_stats(&self) -> CostStats {
        let requests = self.requests.read().unwrap();
        self.compute_cost_stats_inner(&requests)
    }

    fn compute_cost_stats_inner(&self, requests: &[LlmRequest]) -> CostStats {
        let count = requests.len() as f64;
        if count == 0.0 {
            return CostStats::default();
        }
        let total_usd: f64 = requests.iter().map(|r| r.cost_usd).sum();
        let mut by_model: HashMap<String, f64> = HashMap::new();
        let mut by_provider: HashMap<String, f64> = HashMap::new();
        let mut by_user: HashMap<String, f64> = HashMap::new();

        for r in requests.iter() {
            *by_model.entry(r.model.clone()).or_default() += r.cost_usd;
            let provider_key = format!("{:?}", r.provider).to_lowercase();
            *by_provider.entry(provider_key).or_default() += r.cost_usd;
            if let Some(user) = &r.user_id {
                *by_user.entry(user.clone()).or_default() += r.cost_usd;
            }
        }

        CostStats {
            total_usd,
            by_model,
            by_provider,
            by_user,
            avg_per_request: total_usd / count,
        }
    }

    pub fn compute_latency_stats(&self) -> LatencyStats {
        let requests = self.requests.read().unwrap();
        self.compute_latency_stats_inner(&requests)
    }

    fn compute_latency_stats_inner(&self, requests: &[LlmRequest]) -> LatencyStats {
        if requests.is_empty() {
            return LatencyStats::default();
        }
        let mut latencies: Vec<u64> = requests.iter().map(|r| r.latency_ms).collect();
        latencies.sort_unstable();
        let len = latencies.len();
        let avg_ms = latencies.iter().sum::<u64>() as f64 / len as f64;
        let p50_ms = latencies[len / 2];
        let p95_ms = latencies[(len as f64 * 0.95) as usize].min(*latencies.last().unwrap());
        let p99_ms = latencies[(len as f64 * 0.99) as usize].min(*latencies.last().unwrap());
        let max_ms = *latencies.last().unwrap();
        LatencyStats {
            avg_ms,
            p50_ms,
            p95_ms,
            p99_ms,
            max_ms,
        }
    }
}

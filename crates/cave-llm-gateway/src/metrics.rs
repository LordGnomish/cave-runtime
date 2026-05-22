// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus-compatible counter/histogram set for the gateway.
//!
//! cave-metrics scrapes the exposition endpoint mounted in
//! `routes::create_router` (`/metrics`). The data lives in a single
//! lock-protected struct so updates from arbitrary axum handlers stay
//! cheap and consistent.
//!
//! This file deliberately avoids pulling in the upstream Prometheus
//! Rust client — cave-metrics speaks plain text exposition format and we
//! emit exactly that. Keeps the gateway dependency-free of the giant
//! `prometheus-client` crate.

use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;

#[derive(Debug, Default, Clone)]
pub struct ProviderStats {
    pub requests_total: u64,
    pub errors_total: u64,
    pub tokens_in_total: u64,
    pub tokens_out_total: u64,
    pub latency_ms_sum: u64,
    pub latency_ms_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

#[derive(Debug, Default)]
struct MetricsInner {
    providers: BTreeMap<String, ProviderStats>,
}

#[derive(Debug, Default, Clone)]
pub struct GatewayMetrics {
    inner: Arc<Mutex<MetricsInner>>,
}

impl GatewayMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_mut<F: FnOnce(&mut ProviderStats)>(&self, provider: &str, f: F) {
        let mut g = self.inner.lock();
        let entry = g.providers.entry(provider.to_string()).or_default();
        f(entry);
    }

    pub fn record_request(&self, provider: &str) {
        self.with_mut(provider, |s| s.requests_total += 1);
    }

    pub fn record_error(&self, provider: &str) {
        self.with_mut(provider, |s| s.errors_total += 1);
    }

    pub fn record_tokens(&self, provider: &str, tokens_in: u32, tokens_out: u32) {
        self.with_mut(provider, |s| {
            s.tokens_in_total += tokens_in as u64;
            s.tokens_out_total += tokens_out as u64;
        });
    }

    pub fn record_latency_ms(&self, provider: &str, ms: u64) {
        self.with_mut(provider, |s| {
            s.latency_ms_sum = s.latency_ms_sum.saturating_add(ms);
            s.latency_ms_count += 1;
        });
    }

    pub fn record_cache_hit(&self, provider: &str) {
        self.with_mut(provider, |s| s.cache_hits += 1);
    }

    pub fn record_cache_miss(&self, provider: &str) {
        self.with_mut(provider, |s| s.cache_misses += 1);
    }

    /// Snapshot for diagnostics; clones the BTreeMap, holds the lock
    /// only as long as needed.
    pub fn snapshot(&self) -> BTreeMap<String, ProviderStats> {
        self.inner.lock().providers.clone()
    }

    /// Plain-text Prometheus exposition format. Stable line ordering so
    /// scrape diffs are diff-able by humans.
    pub fn render_prometheus(&self) -> String {
        let snap = self.snapshot();
        let mut out = String::new();
        out.push_str("# HELP cave_llm_gateway_requests_total Total LLM requests by provider\n");
        out.push_str("# TYPE cave_llm_gateway_requests_total counter\n");
        for (p, s) in &snap {
            out.push_str(&format!(
                "cave_llm_gateway_requests_total{{provider=\"{}\"}} {}\n",
                p, s.requests_total
            ));
        }
        out.push_str("# HELP cave_llm_gateway_errors_total Total LLM errors by provider\n");
        out.push_str("# TYPE cave_llm_gateway_errors_total counter\n");
        for (p, s) in &snap {
            out.push_str(&format!(
                "cave_llm_gateway_errors_total{{provider=\"{}\"}} {}\n",
                p, s.errors_total
            ));
        }
        out.push_str("# HELP cave_llm_gateway_tokens_total Total tokens consumed by direction\n");
        out.push_str("# TYPE cave_llm_gateway_tokens_total counter\n");
        for (p, s) in &snap {
            out.push_str(&format!(
                "cave_llm_gateway_tokens_total{{provider=\"{}\",direction=\"input\"}} {}\n",
                p, s.tokens_in_total
            ));
            out.push_str(&format!(
                "cave_llm_gateway_tokens_total{{provider=\"{}\",direction=\"output\"}} {}\n",
                p, s.tokens_out_total
            ));
        }
        out.push_str("# HELP cave_llm_gateway_latency_ms_sum Sum of provider latencies\n");
        out.push_str("# TYPE cave_llm_gateway_latency_ms_sum counter\n");
        for (p, s) in &snap {
            out.push_str(&format!(
                "cave_llm_gateway_latency_ms_sum{{provider=\"{}\"}} {}\n",
                p, s.latency_ms_sum
            ));
            out.push_str(&format!(
                "cave_llm_gateway_latency_ms_count{{provider=\"{}\"}} {}\n",
                p, s.latency_ms_count
            ));
        }
        out.push_str("# HELP cave_llm_gateway_cache_hits_total Cache outcomes per provider\n");
        out.push_str("# TYPE cave_llm_gateway_cache_hits_total counter\n");
        for (p, s) in &snap {
            out.push_str(&format!(
                "cave_llm_gateway_cache_hits_total{{provider=\"{}\"}} {}\n",
                p, s.cache_hits
            ));
            out.push_str(&format!(
                "cave_llm_gateway_cache_misses_total{{provider=\"{}\"}} {}\n",
                p, s.cache_misses
            ));
        }
        out
    }
}

/// Process-global singleton so axum handlers and provider wrappers share
/// one counter set without threading a state pointer everywhere.
pub fn global() -> &'static GatewayMetrics {
    static GLOBAL: OnceLock<GatewayMetrics> = OnceLock::new();
    GLOBAL.get_or_init(GatewayMetrics::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_increment_per_provider() {
        let m = GatewayMetrics::new();
        m.record_request("ollama");
        m.record_request("ollama");
        m.record_request("anthropic");
        m.record_tokens("ollama", 100, 50);
        m.record_latency_ms("ollama", 250);
        m.record_error("anthropic");
        m.record_cache_hit("ollama");
        m.record_cache_miss("ollama");

        let snap = m.snapshot();
        assert_eq!(snap["ollama"].requests_total, 2);
        assert_eq!(snap["anthropic"].requests_total, 1);
        assert_eq!(snap["anthropic"].errors_total, 1);
        assert_eq!(snap["ollama"].tokens_in_total, 100);
        assert_eq!(snap["ollama"].tokens_out_total, 50);
        assert_eq!(snap["ollama"].latency_ms_sum, 250);
        assert_eq!(snap["ollama"].latency_ms_count, 1);
        assert_eq!(snap["ollama"].cache_hits, 1);
        assert_eq!(snap["ollama"].cache_misses, 1);
    }

    #[test]
    fn prometheus_format_includes_help_and_type() {
        let m = GatewayMetrics::new();
        m.record_request("ollama");
        let txt = m.render_prometheus();
        assert!(txt.contains("# HELP cave_llm_gateway_requests_total"));
        assert!(txt.contains("# TYPE cave_llm_gateway_requests_total counter"));
        assert!(txt.contains("cave_llm_gateway_requests_total{provider=\"ollama\"} 1"));
    }

    #[test]
    fn empty_metrics_still_emit_header_lines() {
        let m = GatewayMetrics::new();
        let txt = m.render_prometheus();
        assert!(txt.contains("# HELP cave_llm_gateway_requests_total"));
        assert!(txt.contains("# TYPE cave_llm_gateway_errors_total counter"));
    }

    #[test]
    fn global_singleton_returns_same_instance() {
        let a = global() as *const _;
        let b = global() as *const _;
        assert_eq!(a, b);
    }

    #[test]
    fn token_counts_saturate_safely() {
        let m = GatewayMetrics::new();
        m.record_latency_ms("x", u64::MAX);
        m.record_latency_ms("x", 5);
        let snap = m.snapshot();
        assert_eq!(snap["x"].latency_ms_sum, u64::MAX);
        assert_eq!(snap["x"].latency_ms_count, 2);
    }
}

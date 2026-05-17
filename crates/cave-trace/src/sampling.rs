// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Trace sampling strategies.
//!
//! Strategies
//! ──────────
//! Head-based: decision made on the first span.
//!   • `ConstantSampler`      — always/never sample
//!   • `ProbabilisticSampler` — sample X% based on trace-ID hash
//!   • `RateLimitingSampler`  — sample up to N traces per second (token bucket)
//!
//! Tail-based: decision made after all spans collected.
//!   • `TailSampler`          — rule-based; apply rules in priority order
//!
//! Adaptive: automatically adjust rate to hit a target throughput.
//!   • `AdaptiveSampler`      — wraps probabilistic, adjusts rate each period

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::types::{Span, SpanStatus, TraceId};

// ─── Sampler trait ─────────────────────────────────────────────────────────

pub trait Sampler: Send + Sync {
    /// Decide whether to sample a new trace, based on the root span.
    fn should_sample(&self, trace_id: TraceId, root_span: &Span) -> SamplingDecision;

    /// Human-readable description of this sampler.
    fn description(&self) -> &str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingDecision {
    Sample,
    Drop,
}

impl SamplingDecision {
    pub fn is_sample(self) -> bool {
        self == SamplingDecision::Sample
    }
}

// ─── ConstantSampler ───────────────────────────────────────────────────────

pub struct ConstantSampler {
    sample: bool,
}

impl ConstantSampler {
    pub fn always() -> Self { ConstantSampler { sample: true } }
    pub fn never()  -> Self { ConstantSampler { sample: false } }
}

impl Sampler for ConstantSampler {
    fn should_sample(&self, _: TraceId, _: &Span) -> SamplingDecision {
        if self.sample { SamplingDecision::Sample } else { SamplingDecision::Drop }
    }

    fn description(&self) -> &str {
        if self.sample { "constant(always)" } else { "constant(never)" }
    }
}

// ─── ProbabilisticSampler ─────────────────────────────────────────────────

/// Deterministically samples traces at the given rate (0.0 – 1.0) using the
/// trace ID as the source of randomness — identical for all services in a trace.
pub struct ProbabilisticSampler {
    /// Sampling fraction in [0.0, 1.0].
    rate: f64,
    /// Pre-computed threshold: sample if hash ≤ threshold.
    threshold: u64,
}

impl ProbabilisticSampler {
    pub fn new(rate: f64) -> Self {
        let rate = rate.clamp(0.0, 1.0);
        let threshold = (rate * u64::MAX as f64) as u64;
        ProbabilisticSampler { rate, threshold }
    }
}

impl Sampler for ProbabilisticSampler {
    fn should_sample(&self, trace_id: TraceId, _: &Span) -> SamplingDecision {
        let hash = fnv1a_u128(trace_id);
        if hash <= self.threshold {
            SamplingDecision::Sample
        } else {
            SamplingDecision::Drop
        }
    }

    fn description(&self) -> &str {
        // Can't format in trait; caller can call rate()
        "probabilistic"
    }
}

impl ProbabilisticSampler {
    pub fn rate(&self) -> f64 { self.rate }
}

// ─── RateLimitingSampler ──────────────────────────────────────────────────

/// Allow up to `max_traces_per_second` samples using a token-bucket algorithm.
pub struct RateLimitingSampler {
    state: Mutex<RateLimiterState>,
    description: String,
}

struct RateLimiterState {
    max_tps: f64,
    tokens: f64,
    last_refill: Instant,
}

impl RateLimitingSampler {
    pub fn new(max_traces_per_second: f64) -> Self {
        RateLimitingSampler {
            state: Mutex::new(RateLimiterState {
                max_tps: max_traces_per_second,
                tokens: max_traces_per_second,
                last_refill: Instant::now(),
            }),
            description: format!("rate_limiting({} tps)", max_traces_per_second),
        }
    }
}

impl Sampler for RateLimitingSampler {
    fn should_sample(&self, _: TraceId, _: &Span) -> SamplingDecision {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(s.last_refill).as_secs_f64();
        s.tokens = (s.tokens + elapsed * s.max_tps).min(s.max_tps);
        s.last_refill = now;

        if s.tokens >= 1.0 {
            s.tokens -= 1.0;
            SamplingDecision::Sample
        } else {
            SamplingDecision::Drop
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ─── Tail-based sampling ──────────────────────────────────────────────────

/// Rules evaluated against a complete trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TailRule {
    /// Sample if any span has an error status.
    AlwaysOnError,
    /// Sample if trace duration exceeds threshold_ns.
    SlowTrace { threshold_ns: u64 },
    /// Sample if any span's tag matches.
    TagMatch { key: String, value: String },
    /// Sample if the service name matches.
    ServiceMatch { service: String },
    /// Probabilistic fallthrough at the given rate.
    Probabilistic { rate: f64 },
}

impl TailRule {
    fn matches(&self, spans: &[Span]) -> bool {
        match self {
            TailRule::AlwaysOnError => spans.iter().any(|s| s.has_error()),
            TailRule::SlowTrace { threshold_ns } => {
                let start = spans.iter().map(|s| s.start_time_unix_nano).min().unwrap_or(0);
                let end = spans.iter().map(|s| s.end_time_unix_nano).max().unwrap_or(0);
                end.saturating_sub(start) >= *threshold_ns
            }
            TailRule::TagMatch { key, value } => spans.iter().any(|s| {
                s.tags.get(key).map(|v| v.display() == *value).unwrap_or(false)
            }),
            TailRule::ServiceMatch { service } => {
                spans.iter().any(|s| &s.service_name == service)
            }
            TailRule::Probabilistic { rate } => {
                // Use first span's trace_id for determinism
                if let Some(span) = spans.first() {
                    let threshold = (*rate * u64::MAX as f64) as u64;
                    fnv1a_u128(span.trace_id) <= threshold
                } else {
                    false
                }
            }
        }
    }
}

pub struct TailSampler {
    rules: Vec<TailRule>,
}

impl TailSampler {
    pub fn new(rules: Vec<TailRule>) -> Self {
        TailSampler { rules }
    }

    /// Evaluate all rules; sample if any matches (OR semantics).
    pub fn evaluate(&self, spans: &[Span]) -> SamplingDecision {
        if self.rules.iter().any(|r| r.matches(spans)) {
            SamplingDecision::Sample
        } else {
            SamplingDecision::Drop
        }
    }
}

// ─── Adaptive sampler ─────────────────────────────────────────────────────

/// Adjusts probabilistic rate each `period` to approach `target_tps`.
pub struct AdaptiveSampler {
    inner: Mutex<AdaptiveState>,
}

struct AdaptiveState {
    rate: f64,
    target_tps: f64,
    sampled_count: u64,
    period_start: Instant,
    period: Duration,
}

impl AdaptiveSampler {
    pub fn new(initial_rate: f64, target_tps: f64, period: Duration) -> Self {
        AdaptiveSampler {
            inner: Mutex::new(AdaptiveState {
                rate: initial_rate,
                target_tps,
                sampled_count: 0,
                period_start: Instant::now(),
                period,
            }),
        }
    }

    fn current_rate(&self) -> f64 {
        self.inner.lock().unwrap().rate
    }
}

impl Sampler for AdaptiveSampler {
    fn should_sample(&self, trace_id: TraceId, _: &Span) -> SamplingDecision {
        let mut s = self.inner.lock().unwrap();

        // Check if period has elapsed → adjust rate
        let elapsed = s.period_start.elapsed();
        if elapsed >= s.period {
            let elapsed_secs = elapsed.as_secs_f64().max(0.001);
            let observed_tps = s.sampled_count as f64 / elapsed_secs;

            // PID-lite: proportional adjustment
            let ratio = if observed_tps > 0.0 {
                s.target_tps / observed_tps
            } else {
                2.0 // double rate if nothing sampled
            };

            s.rate = (s.rate * ratio).clamp(0.001, 1.0);
            s.sampled_count = 0;
            s.period_start = Instant::now();
        }

        let threshold = (s.rate * u64::MAX as f64) as u64;
        if fnv1a_u128(trace_id) <= threshold {
            s.sampled_count += 1;
            SamplingDecision::Sample
        } else {
            SamplingDecision::Drop
        }
    }

    fn description(&self) -> &str {
        "adaptive"
    }
}

// ─── Config + factory ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SamplingConfig {
    Constant { sample: bool },
    Probabilistic { rate: f64 },
    RateLimiting { max_tps: f64 },
    Adaptive { initial_rate: f64, target_tps: f64, period_secs: f64 },
}

impl Default for SamplingConfig {
    fn default() -> Self {
        SamplingConfig::Constant { sample: true }
    }
}

pub fn build_sampler(config: &SamplingConfig) -> Arc<dyn Sampler + Send + Sync> {
    match config {
        SamplingConfig::Constant { sample } => {
            if *sample {
                Arc::new(ConstantSampler::always())
            } else {
                Arc::new(ConstantSampler::never())
            }
        }
        SamplingConfig::Probabilistic { rate } => {
            Arc::new(ProbabilisticSampler::new(*rate))
        }
        SamplingConfig::RateLimiting { max_tps } => {
            Arc::new(RateLimitingSampler::new(*max_tps))
        }
        SamplingConfig::Adaptive { initial_rate, target_tps, period_secs } => {
            Arc::new(AdaptiveSampler::new(
                *initial_rate,
                *target_tps,
                Duration::from_secs_f64(*period_secs),
            ))
        }
    }
}

// ─── Hash helper ───────────────────────────────────────────────────────────

fn fnv1a_u128(v: u128) -> u64 {
    let bytes = v.to_le_bytes();
    let mut h = 0xcbf29ce484222325u64;
    for &b in &bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x00000100000001b3);
    }
    h
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashMap;

    fn stub_span(trace_id: TraceId) -> Span {
        Span {
            trace_id,
            span_id: 1,
            parent_span_id: None,
            operation_name: "op".into(),
            service_name: "svc".into(),
            start_time_unix_nano: 0,
            end_time_unix_nano: 1_000_000,
            duration_ns: 1_000_000,
            status: SpanStatus::Ok,
            kind: SpanKind::Server,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
            tenant_id: "default".into(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        }
    }

    #[test]
    fn constant_always_samples() {
        let s = ConstantSampler::always();
        assert!(s.should_sample(1, &stub_span(1)).is_sample());
    }

    #[test]
    fn constant_never_drops() {
        let s = ConstantSampler::never();
        assert!(!s.should_sample(1, &stub_span(1)).is_sample());
    }

    #[test]
    fn probabilistic_deterministic() {
        let s = ProbabilisticSampler::new(0.5);
        let d1 = s.should_sample(42, &stub_span(42));
        let d2 = s.should_sample(42, &stub_span(42));
        assert_eq!(d1, d2);
    }

    #[test]
    fn probabilistic_zero_rate_drops_all() {
        let s = ProbabilisticSampler::new(0.0);
        for i in 0..100u128 {
            assert!(!s.should_sample(i, &stub_span(i)).is_sample());
        }
    }

    #[test]
    fn probabilistic_full_rate_samples_all() {
        let s = ProbabilisticSampler::new(1.0);
        for i in 0..100u128 {
            assert!(s.should_sample(i, &stub_span(i)).is_sample());
        }
    }

    #[test]
    fn tail_sampler_error_rule() {
        let mut span = stub_span(1);
        span.status = SpanStatus::Error;
        let ts = TailSampler::new(vec![TailRule::AlwaysOnError]);
        assert!(ts.evaluate(&[span]).is_sample());
    }

    #[test]
    fn tail_sampler_slow_trace_rule() {
        let mut span = stub_span(1);
        span.start_time_unix_nano = 0;
        span.end_time_unix_nano = 2_000_000_000; // 2 s
        let ts = TailSampler::new(vec![TailRule::SlowTrace { threshold_ns: 1_000_000_000 }]);
        assert!(ts.evaluate(&[span]).is_sample());
    }

    #[test]
    fn rate_limiting_allows_burst() {
        let s = RateLimitingSampler::new(10.0);
        let sampled: usize = (0..10)
            .filter(|i| s.should_sample(*i as u128, &stub_span(*i)).is_sample())
            .count();
        assert_eq!(sampled, 10);
    }
}

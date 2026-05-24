// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Trace plugin — OpenTelemetry span emitter.
//!
//! Upstream `plugin/trace/trace.go` wraps each query in an OpenTracing /
//! OpenTelemetry span and forwards trace context downstream. cave-dns ships
//! a structured-log span fallback (the OTel SDK lives in cave-trace) and
//! tracks span counters so operators can verify the plugin is taking
//! traffic.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::info;

use crate::{
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceConfig {
    pub endpoint: String,
    pub service_name: String,
    /// Sample rate in [0, 1].
    pub sample_rate: f64,
    /// Emit one structured-log line per span (when the OTel exporter is not
    /// wired). Defaults to true so traces surface in `tracing` logs.
    pub fallback_log: bool,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4317".into(),
            service_name: "cave-dns".into(),
            sample_rate: 1.0,
            fallback_log: true,
        }
    }
}

pub struct TracePlugin {
    config: TraceConfig,
    spans_emitted: AtomicU64,
    spans_dropped: AtomicU64,
}

impl TracePlugin {
    pub fn new(mut config: TraceConfig) -> Self {
        config.sample_rate = config.sample_rate.clamp(0.0, 1.0);
        Self {
            config,
            spans_emitted: AtomicU64::new(0),
            spans_dropped: AtomicU64::new(0),
        }
    }

    pub fn spans_emitted(&self) -> u64 {
        self.spans_emitted.load(Ordering::Relaxed)
    }

    pub fn spans_dropped(&self) -> u64 {
        self.spans_dropped.load(Ordering::Relaxed)
    }

    pub fn endpoint(&self) -> &str {
        &self.config.endpoint
    }

    pub fn service_name(&self) -> &str {
        &self.config.service_name
    }

    pub fn sample_rate(&self) -> f64 {
        self.config.sample_rate
    }

    /// Deterministic sampler over `(request_id, sample_rate)` so tests can
    /// pin the sample decision without flakiness.
    pub fn should_sample(&self, request_id: u32) -> bool {
        if self.config.sample_rate >= 1.0 {
            return true;
        }
        if self.config.sample_rate <= 0.0 {
            return false;
        }
        let bucket = (request_id as f64 / u32::MAX as f64).clamp(0.0, 1.0);
        bucket < self.config.sample_rate
    }
}

#[async_trait]
impl Plugin for TracePlugin {
    fn name(&self) -> &str {
        "trace"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let request_id = ctx.request.id() as u32;
        let sample = self.should_sample(request_id);
        let outcome = next.run(ctx).await;
        if sample {
            self.spans_emitted.fetch_add(1, Ordering::Relaxed);
            if self.config.fallback_log {
                info!(
                    target: "cave_dns::trace",
                    service = %self.config.service_name,
                    request_id = request_id,
                    rcode = ?ctx.response.response_code(),
                    latency_ms = ctx.elapsed_ms(),
                    "dns.query.span"
                );
            }
        } else {
            self.spans_dropped.fetch_add(1, Ordering::Relaxed);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_default_samples_everything() {
        let t = TracePlugin::new(TraceConfig::default());
        assert!(t.should_sample(0));
        assert!(t.should_sample(u32::MAX));
        assert_eq!(t.sample_rate(), 1.0);
        assert_eq!(t.name(), "trace");
        assert_eq!(t.endpoint(), "http://localhost:4317");
        assert_eq!(t.service_name(), "cave-dns");
    }

    #[test]
    fn trace_zero_rate_drops_all() {
        let t = TracePlugin::new(TraceConfig {
            sample_rate: 0.0,
            ..Default::default()
        });
        assert!(!t.should_sample(0));
        assert!(!t.should_sample(u32::MAX / 2));
    }

    #[test]
    fn trace_half_rate_splits_buckets() {
        let t = TracePlugin::new(TraceConfig {
            sample_rate: 0.5,
            ..Default::default()
        });
        assert!(t.should_sample(0));
        assert!(t.should_sample(u32::MAX / 4));
        assert!(!t.should_sample(u32::MAX - 1));
    }

    #[test]
    fn trace_rate_clamps_to_unit_interval() {
        let t = TracePlugin::new(TraceConfig {
            sample_rate: 1.5,
            ..Default::default()
        });
        assert_eq!(t.sample_rate(), 1.0);
        let t2 = TracePlugin::new(TraceConfig {
            sample_rate: -0.2,
            ..Default::default()
        });
        assert_eq!(t2.sample_rate(), 0.0);
    }

    #[test]
    fn trace_counters_start_at_zero() {
        let t = TracePlugin::new(TraceConfig::default());
        assert_eq!(t.spans_emitted(), 0);
        assert_eq!(t.spans_dropped(), 0);
    }
}

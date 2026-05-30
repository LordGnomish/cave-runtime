// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Synthetic trace generator — a faithful in-crate line-port of Jaeger's
//! `tracegen` load tester (jaegertracing/jaeger v1.52.0,
//! `internal/tracegen/config.go` + `internal/tracegen/worker.go`).
//!
//! Upstream `tracegen` is shipped as a standalone binary (`cmd/tracegen`)
//! that emits synthetic traces over OTLP for load testing. The *binary*,
//! its CLI flag plumbing, and the OTLP exporter are out of scope (operational
//! packaging — see manifest [[scope_cuts]] standalone-cli-binaries). What is
//! genuinely in-crate is the deterministic **span-shape construction
//! algorithm**: each trace is a root SERVER span `"lets-go"` carrying the two
//! canonical peer attributes, plus `child_spans` child CLIENT spans named
//! `child-span-NN`. Every span lasts `FAKE_SPAN_DURATION` (123 µs), child `c`
//! starts at `start + c*FAKE_SPAN_DURATION`, and the parent's total duration
//! is `child_spans * FAKE_SPAN_DURATION`. Per-child attributes cycle their
//! key index modulo `attr_keys` and value index modulo `attr_values`, exactly
//! as `worker.simulateChildSpans` does upstream.
//!
//! This module ports that pure logic so cave-trace can produce reproducible
//! synthetic traces in-process (tests, demos, store-load benchmarks) without
//! the standalone binary or any network exporter.

use std::collections::HashMap;

use crate::types::{Span, SpanId, SpanKind, SpanStatus, TagValue, TraceId};

/// `fakeSpanDuration = 123 * time.Microsecond` — worker.go constant.
pub const FAKE_SPAN_DURATION_NS: u64 = 123_000;

/// Port of `internal/tracegen/config.go::Config` — the test-scenario knobs.
///
/// Defaults mirror the upstream flag defaults (see `Config.Flags`).
#[derive(Debug, Clone)]
pub struct TracegenConfig {
    /// Number of workers (independent generation streams).
    pub workers: usize,
    /// Number of unique service-name suffixes (`tracegen-NN`); one service per trace.
    pub services: usize,
    /// Number of traces to generate per worker.
    pub traces: usize,
    /// Number of child spans per trace.
    pub child_spans: usize,
    /// Number of attributes per child span.
    pub attributes: usize,
    /// Number of distinct attribute keys to cycle through.
    pub attr_keys: usize,
    /// Number of distinct attribute values to cycle through.
    pub attr_values: usize,
    /// Whether to set the `jaeger.debug` flag on the root span.
    pub debug: bool,
    /// Whether to set the `jaeger.firehose` flag on the root span.
    pub firehose: bool,
    /// Service-name prefix.
    pub service: String,
}

impl Default for TracegenConfig {
    fn default() -> Self {
        // Mirrors Config.Flags upstream defaults.
        TracegenConfig {
            workers: 1,
            services: 1,
            traces: 1,
            child_spans: 1,
            attributes: 11,
            attr_keys: 97,
            attr_values: 1000,
            debug: false,
            firehose: false,
            service: "tracegen".to_string(),
        }
    }
}

impl TracegenConfig {
    /// Construct a fresh stateful [`Generator`] carrying this config.
    ///
    /// Equivalent to upstream constructing a `worker` with zeroed
    /// `attrKeyNo` / `attrValNo` / `traceNo` counters.
    pub fn generator(&self) -> Generator {
        self.generator_for_worker(0)
    }

    /// Construct a generator for worker `worker_id` — ports the per-worker
    /// `worker.id` field. The id is folded into the deterministic id stream so
    /// distinct workers never mint colliding trace/span ids.
    pub fn generator_for_worker(&self, worker_id: usize) -> Generator {
        Generator {
            config: self.clone(),
            attr_key_no: 0,
            attr_val_no: 0,
            // Each worker owns a disjoint slab of the counter space so trace
            // and span ids are globally unique across the whole scenario.
            id_counter: (worker_id as u64).wrapping_mul(1_000_000_000),
        }
    }
}

/// Stateful per-worker generator (ports the mutable counters on the upstream
/// `worker` struct: `attrKeyNo`, `attrValNo`, plus a deterministic id source).
///
/// Identifiers are produced deterministically from a monotonically increasing
/// counter so output is fully reproducible — the live collector path mints
/// real random ids; `tracegen` only needs distinct, well-formed ids.
pub struct Generator {
    config: TracegenConfig,
    attr_key_no: usize,
    attr_val_no: usize,
    id_counter: u64,
}

impl Generator {
    fn next_trace_id(&mut self) -> TraceId {
        self.id_counter += 1;
        // Spread the counter into the high bits so ids look trace-like and
        // never collide with the per-span id stream below.
        (self.id_counter as u128) << 64 | 0x6361_7665_7472_6163
    }

    fn next_span_id(&mut self) -> SpanId {
        self.id_counter += 1;
        // Salt with a fixed nonzero constant so a span id is never 0.
        self.id_counter.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1
    }

    /// Port of `worker.simulateChildSpans` attribute generation: build the
    /// `attributes`-sized attribute map for one child, advancing the cyclic
    /// key/value counters by `attr_keys` / `attr_values`.
    fn next_child_attributes(&mut self) -> HashMap<String, TagValue> {
        let mut attrs = HashMap::new();
        for _ in 0..self.config.attributes {
            let key = format!("attr_{:02}", self.attr_key_no);
            let val = format!("val_{:02}", self.attr_val_no);
            attrs.insert(key, TagValue::String(val));
            if self.config.attr_keys > 0 {
                self.attr_key_no = (self.attr_key_no + 1) % self.config.attr_keys;
            }
            if self.config.attr_values > 0 {
                self.attr_val_no = (self.attr_val_no + 1) % self.config.attr_values;
            }
        }
        attrs
    }
}

/// Build one synthetic trace — port of `worker.simulateOneTrace` +
/// `worker.simulateChildSpans`.
///
/// Returns the root span followed by `child_spans` child spans, all sharing a
/// freshly minted trace id. `start_ns` is the trace start time in epoch
/// nanoseconds (upstream uses `time.Now()`; we take it as a parameter so the
/// output is deterministic and testable).
pub fn simulate_one_trace(g: &mut Generator, service_name: &str, start_ns: u64) -> Vec<Span> {
    let trace_id = g.next_trace_id();
    let root_id = g.next_span_id();
    let child_spans = g.config.child_spans;

    // ── Root span: "lets-go", SERVER kind, peer attributes. ──
    let mut root_tags: HashMap<String, TagValue> = HashMap::new();
    root_tags.insert(
        "peer.service".to_string(),
        TagValue::String("tracegen-server".to_string()),
    );
    root_tags.insert(
        "peer.host.ipv4".to_string(),
        TagValue::String("1.1.1.1".to_string()),
    );
    if g.config.debug {
        root_tags.insert("jaeger.debug".to_string(), TagValue::Bool(true));
    }
    if g.config.firehose {
        root_tags.insert("jaeger.firehose".to_string(), TagValue::Bool(true));
    }

    // parent total duration = ChildSpans * fakeSpanDuration (the `Pause == 0`
    // branch of simulateOneTrace).
    let total_duration_ns = child_spans as u64 * FAKE_SPAN_DURATION_NS;
    let root = Span {
        trace_id,
        span_id: root_id,
        parent_span_id: None,
        operation_name: "lets-go".to_string(),
        service_name: service_name.to_string(),
        start_time_unix_nano: start_ns,
        end_time_unix_nano: start_ns + total_duration_ns,
        duration_ns: total_duration_ns,
        status: SpanStatus::Unset,
        kind: SpanKind::Server,
        tags: root_tags,
        events: Vec::new(),
        links: Vec::new(),
        resource_attributes: HashMap::new(),
        tenant_id: crate::multi_tenant::DEFAULT_TENANT.to_string(),
        baggage: HashMap::new(),
        log_labels: HashMap::new(),
    };

    let mut spans = Vec::with_capacity(1 + child_spans);
    spans.push(root);

    // ── Child spans: "child-span-NN", CLIENT kind, cyclic attributes. ──
    for c in 0..child_spans {
        let attrs = g.next_child_attributes();
        let child_id = g.next_span_id();
        let child_start = start_ns + (c as u64) * FAKE_SPAN_DURATION_NS;
        spans.push(Span {
            trace_id,
            span_id: child_id,
            parent_span_id: Some(root_id),
            operation_name: format!("child-span-{:02}", c),
            service_name: service_name.to_string(),
            start_time_unix_nano: child_start,
            end_time_unix_nano: child_start + FAKE_SPAN_DURATION_NS,
            duration_ns: FAKE_SPAN_DURATION_NS,
            status: SpanStatus::Unset,
            kind: SpanKind::Client,
            tags: attrs,
            events: Vec::new(),
            links: Vec::new(),
            resource_attributes: HashMap::new(),
            tenant_id: crate::multi_tenant::DEFAULT_TENANT.to_string(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        });
    }

    spans
}

/// Run the whole scenario — port of `internal/tracegen/config.go::Run` plus
/// `worker.simulateTraces`.
///
/// Produces `workers * traces` traces. Each worker emits `traces` traces; the
/// per-trace service name is selected as `worker.simulateTraces` does — round
/// robin over `services` suffixes (`{service}-NN`), or the bare `service`
/// prefix when `services <= 1`. Trace start times are spaced by the trace's
/// own total duration so generated traces never overlap, mirroring the
/// sequential `simulateOneTrace` loop.
pub fn simulate_traces(config: &TracegenConfig) -> Vec<Vec<Span>> {
    let mut out = Vec::with_capacity(config.workers.saturating_mul(config.traces));
    let per_trace_duration = config.child_spans as u64 * FAKE_SPAN_DURATION_NS;

    for worker_id in 0..config.workers {
        let mut g = config.generator_for_worker(worker_id);
        let mut clock: u64 = 0;
        for trace_no in 0..config.traces {
            let service_name = service_name_for(config, trace_no);
            let spans = simulate_one_trace(&mut g, &service_name, clock);
            // Advance the clock past this trace (at least 1ns so successive
            // traces have strictly increasing starts even when child_spans=0).
            clock = clock.saturating_add(per_trace_duration.max(1));
            out.push(spans);
        }
    }

    out
}

/// Port of the service-name selection in `worker.simulateTraces`:
/// when `services > 1`, append a zero-padded suffix cycling over the trace
/// counter (`{service}-NN`); otherwise use the bare prefix.
fn service_name_for(config: &TracegenConfig, trace_no: usize) -> String {
    if config.services > 1 {
        let suffix = trace_no % config.services;
        format!("{}-{:02}", config.service, suffix)
    } else {
        config.service.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_upstream_flag_defaults() {
        let c = TracegenConfig::default();
        assert_eq!(c.attributes, 11);
        assert_eq!(c.attr_keys, 97);
        assert_eq!(c.attr_values, 1000);
        assert_eq!(c.service, "tracegen");
    }

    #[test]
    fn zero_children_yields_single_root() {
        let cfg = TracegenConfig {
            child_spans: 0,
            ..TracegenConfig::default()
        };
        let mut g = cfg.generator();
        let spans = simulate_one_trace(&mut g, "svc", 0);
        assert_eq!(spans.len(), 1);
        assert!(spans[0].parent_span_id.is_none());
        assert_eq!(spans[0].duration_ns, 0);
    }
}

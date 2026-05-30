// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for the synthetic trace generator (jaeger
//! `internal/tracegen/worker.go` + `config.go`, v1.52.0).
//!
//! Ports the pure span-shape construction logic of Jaeger's `tracegen`
//! load tester: a deterministic root SERVER span "lets-go" with the two
//! canonical peer attributes plus N child CLIENT spans "child-span-NN",
//! every span lasting `fakeSpanDuration = 123µs`, the parent's total
//! duration equal to `ChildSpans * fakeSpanDuration`, and child `c`
//! starting at `start + c*fakeSpanDuration`. Attribute keys/values cycle
//! `attr_NN` / `val_NN` modulo AttrKeys / AttrValues. This is the in-crate
//! algorithm; the standalone binary + OTLP exporter stay out of scope.

use cave_trace::tracegen::{TracegenConfig, simulate_one_trace, simulate_traces};
use cave_trace::types::{SpanKind, TagValue};

const FAKE_SPAN_DURATION_NS: u64 = 123_000; // 123 µs

#[test]
fn one_trace_has_root_plus_child_spans() {
    let cfg = TracegenConfig {
        child_spans: 3,
        attributes: 0,
        ..TracegenConfig::default()
    };
    let mut g = cfg.generator();
    let spans = simulate_one_trace(&mut g, "tracegen", 1_000_000_000);

    // 1 root + 3 children.
    assert_eq!(spans.len(), 4);

    let root = spans.iter().find(|s| s.parent_span_id.is_none()).unwrap();
    assert_eq!(root.operation_name, "lets-go");
    assert_eq!(root.kind, SpanKind::Server);
    assert_eq!(root.service_name, "tracegen");

    let children: Vec<_> = spans.iter().filter(|s| s.parent_span_id.is_some()).collect();
    assert_eq!(children.len(), 3);
    for c in &children {
        assert_eq!(c.kind, SpanKind::Client);
        assert_eq!(c.parent_span_id, Some(root.span_id));
        assert_eq!(c.trace_id, root.trace_id);
    }
}

#[test]
fn root_carries_canonical_peer_attributes() {
    let cfg = TracegenConfig {
        child_spans: 1,
        ..TracegenConfig::default()
    };
    let mut g = cfg.generator();
    let spans = simulate_one_trace(&mut g, "svc", 0);
    let root = spans.iter().find(|s| s.parent_span_id.is_none()).unwrap();

    assert_eq!(
        root.tags.get("peer.service"),
        Some(&TagValue::String("tracegen-server".into()))
    );
    assert_eq!(
        root.tags.get("peer.host.ipv4"),
        Some(&TagValue::String("1.1.1.1".into()))
    );
}

#[test]
fn timing_matches_fake_span_duration() {
    let cfg = TracegenConfig {
        child_spans: 4,
        ..TracegenConfig::default()
    };
    let mut g = cfg.generator();
    let start = 5_000_000_000u64;
    let spans = simulate_one_trace(&mut g, "svc", start);

    let root = spans.iter().find(|s| s.parent_span_id.is_none()).unwrap();
    // parent total duration = ChildSpans * fakeSpanDuration
    assert_eq!(root.start_time_unix_nano, start);
    assert_eq!(
        root.duration_ns,
        4 * FAKE_SPAN_DURATION_NS,
        "parent total duration must be ChildSpans * fakeSpanDuration"
    );
    assert_eq!(root.end_time_unix_nano, start + 4 * FAKE_SPAN_DURATION_NS);

    // child c starts at start + c*fakeSpanDuration, lasts fakeSpanDuration.
    let mut children: Vec<_> = spans.iter().filter(|s| s.parent_span_id.is_some()).collect();
    children.sort_by_key(|s| s.start_time_unix_nano);
    for (c, span) in children.iter().enumerate() {
        let child_start = start + (c as u64) * FAKE_SPAN_DURATION_NS;
        assert_eq!(span.operation_name, format!("child-span-{:02}", c));
        assert_eq!(span.start_time_unix_nano, child_start);
        assert_eq!(span.duration_ns, FAKE_SPAN_DURATION_NS);
        assert_eq!(span.end_time_unix_nano, child_start + FAKE_SPAN_DURATION_NS);
    }
}

#[test]
fn child_attributes_cycle_by_key_and_value() {
    // attr_keys=2, attr_values=3, 2 attributes per child, 2 children.
    let cfg = TracegenConfig {
        child_spans: 2,
        attributes: 2,
        attr_keys: 2,
        attr_values: 3,
        ..TracegenConfig::default()
    };
    let mut g = cfg.generator();
    let spans = simulate_one_trace(&mut g, "svc", 0);
    let mut children: Vec<_> = spans
        .iter()
        .filter(|s| s.parent_span_id.is_some())
        .collect();
    children.sort_by_key(|s| s.start_time_unix_nano);

    // First child: keys attr_00, attr_01 ; vals val_00, val_01.
    let c0 = &children[0];
    assert_eq!(c0.tags.get("attr_00"), Some(&TagValue::String("val_00".into())));
    assert_eq!(c0.tags.get("attr_01"), Some(&TagValue::String("val_01".into())));

    // Second child continues the counters: key cycles mod 2 -> attr_00, attr_01;
    // value cycles mod 3 -> val_02, val_00.
    let c1 = &children[1];
    assert_eq!(c1.tags.get("attr_00"), Some(&TagValue::String("val_02".into())));
    assert_eq!(c1.tags.get("attr_01"), Some(&TagValue::String("val_00".into())));
}

#[test]
fn debug_and_firehose_flags_add_root_tags() {
    let cfg = TracegenConfig {
        child_spans: 0,
        debug: true,
        firehose: true,
        ..TracegenConfig::default()
    };
    let mut g = cfg.generator();
    let spans = simulate_one_trace(&mut g, "svc", 0);
    let root = &spans[0];
    assert_eq!(root.tags.get("jaeger.debug"), Some(&TagValue::Bool(true)));
    assert_eq!(root.tags.get("jaeger.firehose"), Some(&TagValue::Bool(true)));
}

#[test]
fn simulate_traces_respects_worker_and_trace_counts() {
    // 2 workers x 3 traces each = 6 traces, each (1 + child_spans) spans.
    let cfg = TracegenConfig {
        workers: 2,
        traces: 3,
        child_spans: 2,
        ..TracegenConfig::default()
    };
    let traces = simulate_traces(&cfg);
    assert_eq!(traces.len(), 6, "workers * traces total traces");
    for t in &traces {
        assert_eq!(t.len(), 3, "1 root + 2 children per trace");
    }
    // Every generated trace must have a unique trace_id.
    let mut ids: Vec<u128> = traces.iter().map(|t| t[0].trace_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 6, "trace_ids must be unique across generated traces");
}

#[test]
fn services_suffix_names_when_multiple() {
    // services=2 -> service names cycle tracegen-00, tracegen-01.
    let cfg = TracegenConfig {
        workers: 1,
        traces: 4,
        child_spans: 0,
        services: 2,
        service: "tracegen".into(),
        ..TracegenConfig::default()
    };
    let traces = simulate_traces(&cfg);
    let names: Vec<&str> = traces.iter().map(|t| t[0].service_name.as_str()).collect();
    assert_eq!(names, vec!["tracegen-00", "tracegen-01", "tracegen-00", "tracegen-01"]);
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gap-closing edge tests for cave-logs.
//!
//! Targets reachable public APIs that lack inline-`#[cfg(test)]` coverage
//! (models, logql::ast/lexer/parser/eval, multitenant boundary helpers,
//! chunk codec edges, limits saturation, label index, loki push selector
//! parser). Focused on failure modes, boundaries, state transitions, and
//! serde round-trips.

use std::collections::HashMap;
use std::time::Duration;

use cave_logs::{
    chunk::{compress, decode_chunk, decompress, encode_chunk, snappy_raw_compress,
            snappy_raw_decompress, HeadChunk},
    index::{ChunkMeta, LabelIndex, StreamKey},
    ingest::loki_push::parse_label_selector,
    limits::{LimitError, LimitsRegistry},
    logql::{
        ast::{
            BinOp, CompareOp, LabelFilter, LabelFilterValue, LabelMatcher, LineFilter,
            MatchOp, MetricQuery, Parser as AstParser, PipelineStage, Query, RangeAgg,
            StreamSelector, VectorAgg,
        },
        eval::{apply_pipeline, labels_match},
        lexer::{Lexer, Token},
        parser::{ParseError, Parser},
    },
    models::{
        Chunk, Codec, Direction, EntryValue, Labels, LogEntry, LogStream, PushRequest,
        TenantLimits,
    },
    multitenant::{
        dry_run_retention, inject_tenant_stream_label, normalize_tenant_label,
        plan_compaction, tenant_from_headers, CompactionPolicy, RetentionPolicy,
        DEFAULT_TENANT, TENANT_LABEL,
    },
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn lbl(pairs: &[(&str, &str)]) -> Labels {
    Labels::new(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

// ── models: Labels ───────────────────────────────────────────────────────────

#[test]
fn labels_selector_sorts_keys_canonically() {
    // Keys must sort alphabetically regardless of insertion order.
    let l = lbl(&[("z", "1"), ("a", "2"), ("m", "3")]);
    let s = l.to_selector();
    assert_eq!(s, r#"{a="2",m="3",z="1"}"#);
}

#[test]
fn labels_fingerprint_is_order_independent() {
    let a = lbl(&[("x", "1"), ("y", "2")]);
    let b = lbl(&[("y", "2"), ("x", "1")]);
    assert_eq!(a.fingerprint(), b.fingerprint());
}

#[test]
fn labels_fingerprint_differs_on_value_change() {
    let a = lbl(&[("env", "prod")]);
    let b = lbl(&[("env", "dev")]);
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn labels_empty_selector_renders_empty_braces() {
    let l = Labels::default();
    assert_eq!(l.to_selector(), "{}");
    assert!(l.is_empty());
    assert_eq!(l.len(), 0);
}

#[test]
fn labels_insert_and_remove_roundtrip() {
    let mut l = lbl(&[("a", "1")]);
    l.insert("b", "2");
    assert_eq!(l.len(), 2);
    let prev = l.remove("a");
    assert_eq!(prev.as_deref(), Some("1"));
    assert_eq!(l.len(), 1);
    assert!(l.get("a").is_none());
}

// ── models: LogEntry ─────────────────────────────────────────────────────────

#[test]
fn log_entry_timestamp_round_trips_to_datetime() {
    let e = LogEntry::new(1_500_000_000_000_000_000, "hello");
    let dt = e.timestamp();
    // 1.5e18 ns => 1.5e9 seconds since epoch (~2017-07-14 02:40:00 UTC).
    assert_eq!(dt.timestamp(), 1_500_000_000);
    assert_eq!(e.size_bytes(), 5);
}

#[test]
fn log_entry_with_metadata_preserves_pairs() {
    let mut md = HashMap::new();
    md.insert("trace_id".to_string(), "abc".to_string());
    let e = LogEntry::new(0, "line").with_metadata(md);
    assert_eq!(e.metadata.get("trace_id").map(String::as_str), Some("abc"));
}

#[test]
fn log_entry_negative_ns_handled_by_chrono_or_defaults() {
    // Defensive: very negative ts should not panic; either parses or returns default.
    let e = LogEntry::new(-1, "x");
    let _ = e.timestamp(); // must not panic
}

// ── models: LogStream push maintains sort order ─────────────────────────────

#[test]
fn log_stream_push_sorts_out_of_order_arrivals() {
    let mut s = LogStream::new(lbl(&[("a", "1")]), "t");
    s.push(LogEntry::new(300, "c"));
    s.push(LogEntry::new(100, "a"));
    s.push(LogEntry::new(200, "b"));
    let ts: Vec<i64> = s.entries.iter().map(|e| e.ts).collect();
    assert_eq!(ts, vec![100, 200, 300]);
    assert_eq!(s.byte_size(), 3);
    assert_eq!(s.fingerprint(), s.labels.fingerprint());
}

// ── models: Direction / Codec serde round-trip ───────────────────────────────

#[test]
fn direction_codec_serde_round_trip() {
    let d = Direction::Forward;
    let s = serde_json::to_string(&d).unwrap();
    assert_eq!(s, r#""forward""#);
    let back: Direction = serde_json::from_str(&s).unwrap();
    assert_eq!(back, Direction::Forward);

    let c = Codec::Zstd;
    let s = serde_json::to_string(&c).unwrap();
    assert_eq!(s, r#""zstd""#);
    let back: Codec = serde_json::from_str(&s).unwrap();
    assert_eq!(back, Codec::Zstd);

    assert_eq!(Direction::default(), Direction::Backward);
    assert_eq!(Codec::default(), Codec::Snappy);
}

// ── models: EntryValue deserialisation edges ─────────────────────────────────

#[test]
fn entry_value_deserialises_two_element_array() {
    let req: PushRequest = serde_json::from_str(
        r#"{"streams":[{"stream":{"app":"x"},"values":[["1000","hello"]]}]}"#,
    )
    .unwrap();
    assert_eq!(req.streams.len(), 1);
    let ev: &EntryValue = &req.streams[0].values[0];
    assert_eq!(ev.ts_ns, 1000);
    assert_eq!(ev.line, "hello");
    assert!(ev.metadata.is_none());
}

#[test]
fn entry_value_deserialises_with_metadata() {
    let req: PushRequest = serde_json::from_str(
        r#"{"streams":[{"stream":{"app":"x"},"values":[["1","hi",{"k":"v"}]]}]}"#,
    )
    .unwrap();
    let ev = &req.streams[0].values[0];
    assert_eq!(ev.metadata.as_ref().unwrap().get("k").map(String::as_str), Some("v"));
}

#[test]
fn entry_value_rejects_non_array() {
    let r: Result<PushRequest, _> = serde_json::from_str(
        r#"{"streams":[{"stream":{},"values":["not-an-array"]}]}"#,
    );
    assert!(r.is_err());
}

#[test]
fn entry_value_rejects_short_array() {
    let r: Result<PushRequest, _> = serde_json::from_str(
        r#"{"streams":[{"stream":{},"values":[["1000"]]}]}"#,
    );
    assert!(r.is_err());
}

#[test]
fn entry_value_rejects_non_string_timestamp() {
    // ts must be a string in Loki wire format
    let r: Result<PushRequest, _> = serde_json::from_str(
        r#"{"streams":[{"stream":{},"values":[[1000,"line"]]}]}"#,
    );
    assert!(r.is_err());
}

// ── models: TenantLimits defaults ────────────────────────────────────────────

#[test]
fn tenant_limits_defaults_are_loki_sane() {
    let t = TenantLimits::default();
    assert_eq!(t.ingestion_rate_bytes, 4 * 1024 * 1024);
    assert_eq!(t.ingestion_burst_bytes, 16 * 1024 * 1024);
    assert_eq!(t.max_streams, 10_000);
    assert_eq!(t.max_entries_per_query, 5_000);
    assert_eq!(t.max_query_range_hours, 24 * 30);
    assert_eq!(t.max_line_size, 256 * 1024);
    assert_eq!(t.retention_hours, 24 * 7);
}

// ── logql::lexer ─────────────────────────────────────────────────────────────

#[test]
fn lexer_handles_all_duration_units() {
    let cases = [
        ("1ns", 1u64),
        ("2us", 2_000),
        ("3ms", 3_000_000),
        ("4s", 4_000_000_000),
        ("5m", 300_000_000_000),
        ("6h", 21_600_000_000_000),
        ("1d", 86_400_000_000_000),
    ];
    for (input, expected_ns) in cases {
        let toks = Lexer::new(input).tokenize().unwrap();
        match toks.as_slice() {
            [Token::DurationLit(n)] => assert_eq!(*n, expected_ns, "input: {}", input),
            other => panic!("unexpected tokens for {}: {:?}", input, other),
        }
    }
}

#[test]
fn lexer_rejects_unterminated_string() {
    let err = Lexer::new(r#"{a="unterminated"#).tokenize();
    assert!(err.is_err());
}

#[test]
fn lexer_rejects_unknown_duration_unit() {
    let err = Lexer::new("5q").tokenize();
    assert!(err.is_err());
}

#[test]
fn lexer_skips_line_comments() {
    let toks = Lexer::new("// just a comment\n{app=\"x\"}").tokenize().unwrap();
    // Comment is consumed; should start with `{`
    assert_eq!(toks.first(), Some(&Token::LBrace));
}

#[test]
fn lexer_handles_escape_sequences() {
    let toks = Lexer::new(r#""line\nbreak""#).tokenize().unwrap();
    match toks.as_slice() {
        [Token::Str(s)] => assert_eq!(s, "line\nbreak"),
        other => panic!("bad: {:?}", other),
    }
}

// ── logql::parser failure modes ──────────────────────────────────────────────

#[test]
fn parser_empty_input_is_eof() {
    let err = Parser::parse_query("").unwrap_err();
    assert!(matches!(err, ParseError::Eof(_)));
}

#[test]
fn parser_unmatched_brace_errors() {
    let err = Parser::parse_query(r#"{app="nginx""#).unwrap_err();
    // missing closing brace — either Unexpected or Eof
    match err {
        ParseError::Unexpected { .. } | ParseError::Eof(_) | ParseError::Lex(_) => {}
        other => panic!("unexpected error variant: {:?}", other),
    }
}

#[test]
fn parser_bare_open_brace_is_eof() {
    let err = Parser::parse_query("{").unwrap_err();
    assert!(matches!(
        err,
        ParseError::Eof(_) | ParseError::Unexpected { .. }
    ));
}

#[test]
fn parser_all_match_op_variants_round_trip() {
    let cases = [
        (r#"{a="b"}"#, MatchOp::Eq),
        (r#"{a!="b"}"#, MatchOp::Neq),
        (r#"{a=~"b.*"}"#, MatchOp::Re),
        (r#"{a!~"b.*"}"#, MatchOp::NotRe),
    ];
    for (input, expected) in cases {
        let q = Parser::parse_query(input).unwrap();
        match q {
            Query::Log(lq) => assert_eq!(lq.selector.matchers[0].op, expected, "{}", input),
            _ => panic!("expected log query for {}", input),
        }
    }
}

#[test]
fn parser_line_filter_variants_all_parse() {
    let q = Parser::parse_query(r#"{a="b"} |= "x" != "y" |~ "z" !~ "w""#).unwrap();
    match q {
        Query::Log(lq) => {
            assert_eq!(lq.pipeline.len(), 4);
            for (i, expected_variant) in [
                "Contains", "NotContains", "Matches", "NotMatches"
            ].iter().enumerate() {
                let actual = match &lq.pipeline[i] {
                    PipelineStage::LineFilter(LineFilter::Contains(_)) => "Contains",
                    PipelineStage::LineFilter(LineFilter::NotContains(_)) => "NotContains",
                    PipelineStage::LineFilter(LineFilter::Matches(_)) => "Matches",
                    PipelineStage::LineFilter(LineFilter::NotMatches(_)) => "NotMatches",
                    other => panic!("stage {} not a line filter: {:?}", i, other),
                };
                assert_eq!(actual, *expected_variant);
            }
        }
        _ => panic!("expected log query"),
    }
}

#[test]
fn parser_quantile_over_time_extracts_q() {
    let q = Parser::parse_query(r#"quantile_over_time(0.95, {a="b"} | unwrap dur [1m])"#).unwrap();
    match q {
        Query::Metric(MetricQuery::RangeAgg(rg)) => match rg.agg {
            RangeAgg::QuantileOverTime(qv) => assert!((qv - 0.95).abs() < 1e-9),
            other => panic!("expected QuantileOverTime, got {:?}", other),
        },
        _ => panic!("expected range agg"),
    }
}

#[test]
fn parser_topk_records_k() {
    let q = Parser::parse_query(r#"topk(7, rate({a="b"}[1m]))"#).unwrap();
    match q {
        Query::Metric(MetricQuery::VectorAgg(va)) => match va.agg {
            VectorAgg::Topk(k) => assert_eq!(k, 7),
            other => panic!("expected Topk, got {:?}", other),
        },
        _ => panic!("expected vector agg"),
    }
}

#[test]
fn parser_binary_precedence_mul_before_add() {
    // rate(..) + rate(..) * rate(..) -> Add with Mul on the right (precedence climbing).
    let q = Parser::parse_query(
        r#"rate({a="b"}[1m]) + rate({c="d"}[1m]) * rate({e="f"}[1m])"#,
    )
    .unwrap();
    match q {
        Query::Metric(MetricQuery::BinaryExpr(be)) => {
            assert_eq!(be.op, BinOp::Add);
            assert!(
                matches!(*be.rhs, MetricQuery::BinaryExpr(ref inner) if inner.op == BinOp::Mul),
                "rhs should be a Mul binary expression (precedence climbing)"
            );
        }
        _ => panic!("expected binary expr"),
    }
}

#[test]
fn parser_numeric_literal_query() {
    let q = Parser::parse_query("42").unwrap();
    assert!(matches!(q, Query::Metric(MetricQuery::Literal(n)) if (n - 42.0).abs() < 1e-9));
}

#[test]
fn parser_sum_without_grouping() {
    let q = Parser::parse_query(r#"sum without (instance) (rate({a="b"}[1m]))"#).unwrap();
    if let Query::Metric(MetricQuery::VectorAgg(va)) = q {
        let g = va.grouping.expect("grouping present");
        assert!(g.without);
        assert_eq!(g.labels, vec!["instance".to_string()]);
    } else {
        panic!("expected vector agg");
    }
}

// ── logql::eval: labels_match edges ──────────────────────────────────────────

#[test]
fn labels_match_neq_satisfied_by_absence() {
    let l = lbl(&[("env", "prod")]);
    let sel = StreamSelector {
        matchers: vec![LabelMatcher {
            name: "missing".into(),
            op: MatchOp::Neq,
            value: "x".into(),
        }],
    };
    // Absence satisfies !=
    assert!(labels_match(&l, &sel));
}

#[test]
fn labels_match_re_against_missing_label_uses_empty_string() {
    let l = lbl(&[("env", "prod")]);
    let sel = StreamSelector {
        matchers: vec![LabelMatcher {
            name: "missing".into(),
            op: MatchOp::Re,
            value: ".*".into(), // matches empty too
        }],
    };
    assert!(labels_match(&l, &sel));
}

#[test]
fn labels_match_invalid_regex_returns_false() {
    let l = lbl(&[("env", "prod")]);
    let sel = StreamSelector {
        matchers: vec![LabelMatcher {
            name: "env".into(),
            op: MatchOp::Re,
            value: "(".into(), // invalid regex
        }],
    };
    assert!(!labels_match(&l, &sel));
}

// ── logql::eval: apply_pipeline ──────────────────────────────────────────────

#[test]
fn apply_pipeline_line_filter_chain_drops_on_first_miss() {
    let labels = lbl(&[("app", "x")]);
    let entry = LogEntry::new(0, "alpha bravo");
    let pipeline = vec![
        PipelineStage::LineFilter(LineFilter::Contains("alpha".into())),
        PipelineStage::LineFilter(LineFilter::NotContains("bravo".into())),
    ];
    // "bravo" is present, NotContains drops it.
    assert!(apply_pipeline(&entry, &labels, &pipeline).is_none());
}

#[test]
fn apply_pipeline_json_parser_extracts_into_labels() {
    let labels = lbl(&[("app", "x")]);
    let entry = LogEntry::new(0, r#"{"level":"error","code":500}"#);
    let pipeline = vec![PipelineStage::Parser(AstParser::Json)];
    let p = apply_pipeline(&entry, &labels, &pipeline).expect("kept");
    assert_eq!(p.labels.get("level").map(String::as_str), Some("error"));
    assert_eq!(p.labels.get("code").map(String::as_str), Some("500"));
    // Original label preserved.
    assert_eq!(p.labels.get("app").map(String::as_str), Some("x"));
}

#[test]
fn apply_pipeline_label_filter_numeric_compare() {
    let labels = lbl(&[("app", "x")]);
    let entry = LogEntry::new(0, r#"{"status":500}"#);
    let pipeline = vec![
        PipelineStage::Parser(AstParser::Json),
        PipelineStage::LabelFilter(LabelFilter {
            label: "status".into(),
            op: CompareOp::Gte,
            value: LabelFilterValue::Float(400.0),
        }),
    ];
    assert!(apply_pipeline(&entry, &labels, &pipeline).is_some());

    let pipeline_lt = vec![
        PipelineStage::Parser(AstParser::Json),
        PipelineStage::LabelFilter(LabelFilter {
            label: "status".into(),
            op: CompareOp::Lt,
            value: LabelFilterValue::Float(400.0),
        }),
    ];
    assert!(apply_pipeline(&entry, &labels, &pipeline_lt).is_none());
}

// ── multitenant: edge boundaries already covered inline; add cross-feature ──

#[test]
fn retention_cutoff_zero_duration_returns_now() {
    let p = RetentionPolicy::new(Duration::from_nanos(0));
    let cut = p.cutoff_ns(&lbl(&[]), 12_345);
    assert_eq!(cut, 12_345);
}

#[test]
fn plan_compaction_target_zero_flushes_per_chunk() {
    // target_bytes=0 → every chunk size >=0 triggers flush after the chunk is added.
    let policy = CompactionPolicy {
        min_chunks: 1,
        max_chunk_bytes: 1024,
        small_chunk_count_trigger: 16,
    };
    let plan = plan_compaction(&[10, 20, 30], 0, &policy);
    assert_eq!(plan.merge_groups.len(), 3);
    assert_eq!(plan.total_compactable_bytes, 60);
}

#[test]
fn tenant_from_headers_picks_first_canonical_match() {
    let mut h = HashMap::new();
    h.insert("X-Scope-OrgID".to_string(), "primary".to_string());
    assert_eq!(tenant_from_headers(&h), "primary");
}

// ── chunk: codec failure / boundary ──────────────────────────────────────────

#[test]
fn chunk_encode_decode_preserves_metadata() {
    let mut md = HashMap::new();
    md.insert("trace".to_string(), "abc".to_string());
    let entries = vec![LogEntry::new(1, "first").with_metadata(md.clone())];
    let bytes = encode_chunk(&entries, Codec::Gzip).unwrap();
    let chunk = Chunk {
        stream_fp: 0,
        tenant: "t".into(),
        min_ts: 1,
        max_ts: 1,
        codec: Codec::Gzip,
        data: bytes,
        num_entries: 1,
        uncompressed_size: 16,
    };
    let decoded = decode_chunk(&chunk).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].metadata.get("trace").map(String::as_str), Some("abc"));
}

#[test]
fn decompress_corrupt_gzip_yields_error() {
    let bogus = b"definitely not gzip\x00\x01\x02";
    let r = decompress(bogus, Codec::Gzip);
    assert!(r.is_err());
}

#[test]
fn codec_none_roundtrip_is_identity() {
    let data = b"raw bytes";
    let c = compress(data, Codec::None).unwrap();
    assert_eq!(c, data);
    let d = decompress(&c, Codec::None).unwrap();
    assert_eq!(d, data);
}

#[test]
fn snappy_raw_handles_empty_input() {
    let compressed = snappy_raw_compress(b"").unwrap();
    let decompressed = snappy_raw_decompress(&compressed).unwrap();
    assert!(decompressed.is_empty());
}

#[test]
fn head_chunk_threshold_age_triggers_flush() {
    // size threshold not met but age=0 immediate threshold (max_age_secs=0).
    let hc = HeadChunk::new(1, "tenant");
    assert!(hc.should_flush(usize::MAX, 0));
}

#[test]
fn head_chunk_empty_min_max_ts_none() {
    let hc = HeadChunk::new(1, "t");
    assert!(hc.is_empty());
    assert!(hc.min_ts().is_none());
    assert!(hc.max_ts().is_none());
}

// ── limits: error → http status ──────────────────────────────────────────────

#[test]
fn limit_error_line_too_long_returns_400() {
    let mut t = TenantLimits::default();
    t.max_line_size = 10;
    let reg = LimitsRegistry::new(t);
    let err = reg.check_line_size("ten", 100).unwrap_err();
    assert_eq!(err.http_status(), 400);
}

#[test]
fn limit_error_too_many_streams_returns_400() {
    let mut t = TenantLimits::default();
    t.max_streams = 2;
    let reg = LimitsRegistry::new(t);
    let err = reg.check_stream_count("ten", 2).unwrap_err();
    assert_eq!(err.http_status(), 400);
    // Ensure the variant is what we expect.
    assert!(matches!(err, LimitError::TooManyStreams { .. }));
}

#[test]
fn limit_zero_rate_means_unlimited() {
    let mut t = TenantLimits::default();
    t.ingestion_rate_bytes = 0;
    let reg = LimitsRegistry::new(t);
    // Large ingest must pass when rate is 0.
    assert!(reg.check_ingestion_rate("ten", 999_999_999).is_ok());
}

#[test]
fn limit_query_range_zero_max_skips_check() {
    let mut t = TenantLimits::default();
    t.max_query_range_hours = 0; // unlimited
    let reg = LimitsRegistry::new(t);
    let r = reg.check_query_limits("t", 100, 0, i64::MAX / 2);
    assert!(r.is_ok());
}

// ── index: prune + bloom guarantees ──────────────────────────────────────────

#[test]
fn label_index_prune_preserves_recent_chunks() {
    let idx = LabelIndex::new();
    let key = StreamKey::new("t", 1);
    idx.add_chunk(ChunkMeta::new(key.clone(), 0, 100, 1, 10, &["old"]));
    idx.add_chunk(ChunkMeta::new(key.clone(), 500, 1000, 1, 10, &["new"]));
    idx.prune_chunks_before(200);
    // Only the newer chunk should remain.
    assert_eq!(idx.chunk_count(), 1);
}

#[test]
fn label_index_chunks_for_stream_filters_by_range_and_tenant() {
    let idx = LabelIndex::new();
    idx.add_chunk(ChunkMeta::new(StreamKey::new("a", 1), 0, 100, 1, 10, &[]));
    idx.add_chunk(ChunkMeta::new(StreamKey::new("a", 1), 200, 300, 1, 10, &[]));
    idx.add_chunk(ChunkMeta::new(StreamKey::new("b", 1), 0, 100, 1, 10, &[]));
    let hits = idx.chunks_for_stream(1, "a", 50, 250);
    // Both 'a' chunks overlap [50,250]; 'b' is excluded by tenant.
    assert_eq!(hits.len(), 2);
}

// ── ingest::loki_push::parse_label_selector ──────────────────────────────────

#[test]
fn loki_selector_supports_all_op_variants_and_quotes() {
    let l = parse_label_selector(r#"{app="nginx",env=~"prod.*",svc!="db",host!~"dev.*"}"#);
    assert_eq!(l.get("app"), Some("nginx"));
    assert_eq!(l.get("env"), Some("prod.*"));
    assert_eq!(l.get("svc"), Some("db"));
    assert_eq!(l.get("host"), Some("dev.*"));
}

#[test]
fn loki_selector_handles_commas_in_quotes() {
    // Comma inside the quoted value must not split the pair.
    let l = parse_label_selector(r#"{msg="hello, world",app="x"}"#);
    assert_eq!(l.get("msg"), Some("hello, world"));
    assert_eq!(l.get("app"), Some("x"));
}

#[test]
fn loki_selector_empty_string_yields_empty_labels() {
    let l = parse_label_selector("");
    assert!(l.is_empty());
}

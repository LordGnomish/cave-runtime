// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-logs — Full Loki-parity log aggregation for the CAVE Unified Runtime.
//!
//! # Features
//!
//! **Ingestion:**
//! - Loki push API (JSON + protobuf+snappy)
//! - Syslog (RFC 5424 + RFC 3164)
//! - OTLP Logs (HTTP/JSON)
//! - Fluentd forward protocol (MessagePack)
//!
//! **LogQL engine (full):**
//! - Stream selectors: `{label="value"}`, `{label=~"regex"}`, `{label!="value"}`, `{label!~"regex"}`
//! - Line filters: `|= "text"`, `!= "text"`, `|~ "regex"`, `!~ "regex"`
//! - Parsers: `| json`, `| logfmt`, `| regexp`, `| pattern`, `| unpack`
//! - Label filters: `| label >= value`
//! - Line format: `| line_format "{{.label}}"`
//! - Label format: `| label_format new=old`
//! - Metric queries: rate, count_over_time, bytes_over_time, bytes_rate, absent_over_time
//! - Vector aggregations: sum, avg, min, max, count, stddev, stdvar, topk, bottomk, quantile
//! - Binary operations
//!
//! **Storage:**
//! - Chunk-based with gzip / snappy / lz4 / zstd compression
//! - Bloom filter index for fast line matching
//! - Label inverted index for stream selection
//! - Retention and compaction
//! - Multi-tenancy via X-Scope-OrgID
//!
//! **API (Loki HTTP API):**
//! - POST /loki/api/v1/push
//! - GET  /loki/api/v1/query
//! - GET  /loki/api/v1/query_range
//! - GET  /loki/api/v1/labels
//! - GET  /loki/api/v1/label/{name}/values
//! - GET  /loki/api/v1/series
//! - GET  /loki/api/v1/index/stats
//! - GET  /loki/api/v1/tail (WebSocket)
//! - GET  /ready, /metrics

pub mod chunk;
pub mod index;
pub mod ingest;
pub mod ingester_rf1;
pub mod limits;
pub mod logql;
pub mod models;
pub mod multitenant;
pub mod routes;
pub mod shipper;
pub mod store;
pub mod tail;
pub mod tsdb_index;

pub use limits::LimitsRegistry;
pub use routes::{AppState, router};
pub use store::LogStore;

/// Create a fully initialised `AppState` with default configuration.
pub fn default_state() -> AppState {
    AppState {
        store: LogStore::new(),
        limits: LimitsRegistry::with_defaults(),
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;
    use crate::chunk::{
        compress, decode_chunk, decompress, encode_chunk, snappy_raw_compress,
        snappy_raw_decompress,
    };
    use crate::index::{ChunkMeta, LabelIndex, StreamKey};
    use crate::models::{Codec, Direction, Labels, LogEntry, TenantLimits};
    use crate::multitenant::{
        CompactionPolicy, DEFAULT_TENANT, RetentionPolicy, TENANT_LABEL, dry_run_retention,
        inject_tenant_stream_label, normalize_tenant_label, plan_compaction, tenant_from_headers,
    };
    use std::collections::HashMap;
    use std::time::Duration;

    fn lbl(pairs: &[(&str, &str)]) -> Labels {
        Labels::new(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    // ─── Chunk codec roundtrip + helpers ─────────────────────

    #[test]
    fn chunk_codec_gzip_roundtrip_preserves_entries() {
        let entries = vec![
            LogEntry::new(100, "line one"),
            LogEntry::new(200, "line two"),
            LogEntry::new(300, "another line"),
        ];
        let bytes = encode_chunk(&entries, Codec::Gzip).unwrap();
        let chunk = crate::models::Chunk {
            stream_fp: 1,
            tenant: "t".to_string(),
            min_ts: 100,
            max_ts: 300,
            num_entries: 3,
            codec: Codec::Gzip,
            data: bytes,
            uncompressed_size: 100,
        };
        let decoded = decode_chunk(&chunk).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].line, "line one");
    }

    #[test]
    fn chunk_codec_snappy_roundtrip() {
        let data = b"the quick brown fox jumps over the lazy dog".repeat(20);
        let compressed = compress(&data, Codec::Snappy).unwrap();
        let decompressed = decompress(&compressed, Codec::Snappy).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn chunk_codec_lz4_roundtrip() {
        let data = b"alpha bravo charlie delta echo foxtrot golf hotel".repeat(10);
        let compressed = compress(&data, Codec::Lz4).unwrap();
        let decompressed = decompress(&compressed, Codec::Lz4).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn chunk_codec_zstd_roundtrip() {
        let data = b"compressible data ".repeat(50);
        let compressed = compress(&data, Codec::Zstd).unwrap();
        let decompressed = decompress(&compressed, Codec::Zstd).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn chunk_snappy_raw_helpers_roundtrip() {
        let data = b"loki snappy raw frame".to_vec();
        let compressed = snappy_raw_compress(&data).unwrap();
        let decompressed = snappy_raw_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    // ─── LogStore: query direction + retention + flush_all ───

    #[test]
    fn store_query_backward_returns_newest_first() {
        let store = LogStore::new();
        let labels = lbl(&[("svc", "app")]);
        let entries: Vec<LogEntry> = (0..5)
            .map(|i| LogEntry::new(1000 + i, format!("line-{i}")))
            .collect();
        store.push("t", labels, entries).unwrap();
        let fps = store.matching_fps("t", |_| true);
        let res = store.query_entries("t", &fps, 0, i64::MAX, 100, Direction::Backward);
        assert!(!res.is_empty());
        let lines: Vec<&str> = res[0].2.iter().map(|e| e.line.as_str()).collect();
        // First entry must be the latest one (line-4)
        assert_eq!(lines[0], "line-4");
        // Sequence is strictly decreasing
        assert_eq!(
            lines,
            vec!["line-4", "line-3", "line-2", "line-1", "line-0"]
        );
    }

    #[test]
    fn store_query_limit_truncates_results() {
        let store = LogStore::new();
        let entries: Vec<LogEntry> = (0..10).map(|i| LogEntry::new(i, "x")).collect();
        store.push("t", lbl(&[("a", "b")]), entries).unwrap();
        let fps = store.matching_fps("t", |_| true);
        let res = store.query_entries("t", &fps, 0, i64::MAX, 3, Direction::Forward);
        assert_eq!(res[0].2.len(), 3);
    }

    #[test]
    fn store_flush_all_seals_head_chunks() {
        let store = LogStore::new();
        store
            .push("t", lbl(&[("a", "1")]), vec![LogEntry::new(1, "l")])
            .unwrap();
        store
            .push("t", lbl(&[("a", "2")]), vec![LogEntry::new(1, "l")])
            .unwrap();
        let pre_chunks = store.stats().streams; // chunks not necessarily sealed yet
        let _ = pre_chunks;
        store.flush_all().unwrap();
        // After flush, the chunks are sealed and stored in the chunk store.
        let s = store.stats();
        assert!(s.chunks >= 2);
    }

    #[test]
    fn store_series_returns_labels_for_fps() {
        let store = LogStore::new();
        store
            .push("t", lbl(&[("env", "prod")]), vec![LogEntry::new(0, "l")])
            .unwrap();
        let fps = store.matching_fps("t", |_| true);
        let series = store.series("t", &fps);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].get("env"), Some("prod"));
    }

    #[test]
    fn store_dedup_entries_removes_identical_ts_line_pairs() {
        let mut entries = vec![
            LogEntry::new(100, "same"),
            LogEntry::new(100, "same"),
            LogEntry::new(100, "different"),
            LogEntry::new(200, "same"),
        ];
        LogStore::dedup_entries(&mut entries);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn store_bytes_over_buckets_aggregates_payload_size() {
        let store = LogStore::new();
        let step = 1_000_000_000i64;
        let entries: Vec<LogEntry> = (0..3).map(|i| LogEntry::new(i * step / 4, "msg")).collect();
        store.push("t", lbl(&[("a", "b")]), entries).unwrap();
        let fps = store.matching_fps("t", |_| true);
        let buckets = store.bytes_over_buckets("t", &fps, 0, step, step);
        let total: f64 = buckets.iter().map(|(_, b)| b).sum();
        assert!(total > 0.0);
    }

    // ─── LabelIndex isolation + bloom + prune ────────────────

    #[test]
    fn label_index_multi_tenant_isolation() {
        let idx = LabelIndex::new();
        let lblset = lbl(&[("app", "shared")]);
        idx.index_stream("tenant-a", 1, &lblset);
        idx.index_stream("tenant-b", 2, &lblset);
        let fps_a = idx.streams_for_label_value("app", "shared", "tenant-a");
        let fps_b = idx.streams_for_label_value("app", "shared", "tenant-b");
        assert_eq!(fps_a, vec![1]);
        assert_eq!(fps_b, vec![2]);
    }

    #[test]
    fn label_index_prune_chunks_before_drops_old_metas() {
        let idx = LabelIndex::new();
        let key = StreamKey::new("t", 1);
        idx.add_chunk(ChunkMeta::new(key.clone(), 100, 500, 10, 1024, &["a", "b"]));
        idx.add_chunk(ChunkMeta::new(key, 1000, 2000, 10, 1024, &["c"]));
        assert_eq!(idx.chunk_count(), 2);
        idx.prune_chunks_before(800);
        assert_eq!(idx.chunk_count(), 1);
    }

    #[test]
    fn label_index_chunk_might_contain_uses_bloom() {
        let key = StreamKey::new("t", 1);
        let meta = ChunkMeta::new(key, 100, 500, 10, 0, &["error: connection refused"]);
        assert!(meta.might_contain("error: connection refused"));
        // Missing line is unlikely to be reported (bloom can have false positives but
        // a near-miss against an unrelated line should be very low rate).
    }

    #[test]
    fn label_index_remove_stream_clears_indexes() {
        let idx = LabelIndex::new();
        let lblset = lbl(&[("removable", "yes")]);
        idx.index_stream("t", 99, &lblset);
        assert!(!idx.label_values("removable", Some("t")).is_empty());
        idx.remove_stream("t", 99, &lblset);
        assert!(idx.label_values("removable", Some("t")).is_empty());
    }

    // ─── Limits ───────────────────────────────────────────────

    #[test]
    fn limits_ingestion_rate_burst_then_refuse() {
        let mut tl = TenantLimits::default();
        tl.ingestion_rate_bytes = 1000;
        tl.ingestion_burst_bytes = 1000;
        let mut reg = LimitsRegistry::new(tl);
        // Need mutability — get the inner Arc out
        let mr = std::sync::Arc::get_mut(&mut reg).unwrap();
        mr.set_tenant_limits(
            "burst-tenant",
            TenantLimits {
                ingestion_rate_bytes: 1000,
                ingestion_burst_bytes: 1000,
                ..TenantLimits::default()
            },
        );
        // First request consumes the burst
        assert!(reg.check_ingestion_rate("burst-tenant", 800).is_ok());
        // Second request would exceed the remaining tokens
        assert!(reg.check_ingestion_rate("burst-tenant", 500).is_err());
    }

    #[test]
    fn limits_query_range_clamps_to_max_hours() {
        let mut tl = TenantLimits::default();
        tl.max_query_range_hours = 24;
        let reg = LimitsRegistry::new(tl);
        let one_hour_ns = 3_600_000_000_000i64;
        // 25-hour window must be rejected
        let r = reg.check_query_limits("t", 100, 0, 25 * one_hour_ns);
        assert!(r.is_err());
    }

    #[test]
    fn limits_query_limit_clamped_to_max_entries() {
        let mut tl = TenantLimits::default();
        tl.max_entries_per_query = 50;
        let reg = LimitsRegistry::new(tl);
        let effective = reg.check_query_limits("t", 9999, 0, 1).unwrap();
        assert_eq!(effective, 50);
    }

    #[test]
    fn limits_stream_count_rejects_above_max() {
        let mut tl = TenantLimits::default();
        tl.max_streams = 5;
        let reg = LimitsRegistry::new(tl);
        assert!(reg.check_stream_count("t", 4).is_ok());
        assert!(reg.check_stream_count("t", 5).is_err());
    }

    #[test]
    fn limits_line_size_rejects_oversized() {
        let mut tl = TenantLimits::default();
        tl.max_line_size = 100;
        let reg = LimitsRegistry::new(tl);
        assert!(reg.check_line_size("t", 50).is_ok());
        assert!(reg.check_line_size("t", 200).is_err());
    }

    #[test]
    fn limits_error_http_status_rate_limited_429() {
        let mut tl = TenantLimits::default();
        tl.ingestion_rate_bytes = 10;
        tl.ingestion_burst_bytes = 10;
        let reg = LimitsRegistry::new(tl);
        let _ = reg.check_ingestion_rate("t", 10); // drain burst
        let err = reg.check_ingestion_rate("t", 10).unwrap_err();
        assert_eq!(err.http_status(), 429);
    }

    // ─── Multi-tenant primitives ─────────────────────────────

    #[test]
    fn tenant_from_headers_case_insensitive_and_trim() {
        let h: HashMap<String, String> = [("x-scope-orgid".to_string(), "  acme  ".to_string())]
            .into_iter()
            .collect();
        assert_eq!(tenant_from_headers(&h), "acme");
    }

    #[test]
    fn tenant_from_headers_missing_returns_default() {
        let h: HashMap<String, String> = HashMap::new();
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    #[test]
    fn tenant_from_headers_empty_value_returns_default() {
        let h: HashMap<String, String> = [("X-Scope-OrgID".to_string(), "   ".to_string())]
            .into_iter()
            .collect();
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    #[test]
    fn inject_tenant_overwrites_spoofed_label() {
        let mut labels = lbl(&[(TENANT_LABEL, "evil")]);
        inject_tenant_stream_label(&mut labels, "good");
        assert_eq!(labels.get(TENANT_LABEL), Some("good"));
    }

    #[test]
    fn normalize_tenant_returns_clone_with_tenant() {
        let labels = lbl(&[("app", "x")]);
        let normalized = normalize_tenant_label(&labels, "tenant-1");
        assert_eq!(normalized.get(TENANT_LABEL), Some("tenant-1"));
        assert_eq!(normalized.get("app"), Some("x"));
    }

    // ─── Retention / compaction policy ───────────────────────

    #[test]
    fn retention_override_short_circuits_on_first_match() {
        let p = RetentionPolicy::new(Duration::from_secs(7 * 24 * 3600))
            .with_override("env", "prod", Duration::from_secs(30 * 24 * 3600))
            .with_override("env", "dev", Duration::from_secs(24 * 3600));
        let prod = lbl(&[("env", "prod")]);
        let dev = lbl(&[("env", "dev")]);
        let other = lbl(&[("env", "qa")]);
        assert_eq!(p.for_stream(&prod), Duration::from_secs(30 * 24 * 3600));
        assert_eq!(p.for_stream(&dev), Duration::from_secs(24 * 3600));
        assert_eq!(p.for_stream(&other), Duration::from_secs(7 * 24 * 3600));
    }

    #[test]
    fn retention_cutoff_saturates_at_zero_for_huge_durations() {
        let p = RetentionPolicy::new(Duration::from_secs(u64::MAX / 2));
        let labels = lbl(&[]);
        let cut = p.cutoff_ns(&labels, 1_000_000);
        assert_eq!(cut, 0);
    }

    #[test]
    fn dry_run_retention_counts_old_entries() {
        let p = RetentionPolicy::new(Duration::from_secs(100));
        let now_ns = 1_000_000_000_000i64;
        let cutoff = now_ns - 100 * 1_000_000_000;
        let entries = vec![
            LogEntry::new(cutoff - 1_000_000_000, "old"),
            LogEntry::new(cutoff + 1_000_000_000, "new"),
            LogEntry::new(cutoff - 5_000_000_000, "ancient"),
        ];
        let plan = dry_run_retention(&lbl(&[]), &entries, &p, now_ns);
        assert_eq!(plan.deletable_entries, 2);
        assert_eq!(plan.inspected_entries, 3);
    }

    #[test]
    fn plan_compaction_skips_oversized_chunks() {
        let policy = CompactionPolicy {
            min_chunks: 2,
            max_chunk_bytes: 1_000_000,
            small_chunk_count_trigger: 4,
        };
        let chunk_sizes = vec![100, 200, 2_000_000, 300, 400];
        let plan = plan_compaction(&chunk_sizes, 5_000_000, &policy);
        // The mid-stream 2MB chunk is skipped — we should NOT see it included.
        for group in &plan.merge_groups {
            assert!(!group.contains(&2));
        }
    }

    #[test]
    fn plan_compaction_groups_small_chunks_when_target_reached() {
        let policy = CompactionPolicy::default();
        let chunk_sizes = vec![100, 100, 100, 100];
        let plan = plan_compaction(&chunk_sizes, 350, &policy);
        // 4 chunks summing to 400 — group when cum >= 350 → first 4 indices grouped
        assert_eq!(plan.merge_groups.len(), 1);
        assert_eq!(plan.total_compactable_bytes, 400);
    }

    // ─── Push API round-trip via store ───────────────────────

    #[test]
    fn store_handles_separate_streams_independently() {
        let store = LogStore::new();
        store
            .push("t", lbl(&[("svc", "a")]), vec![LogEntry::new(1, "x")])
            .unwrap();
        store
            .push("t", lbl(&[("svc", "b")]), vec![LogEntry::new(1, "y")])
            .unwrap();
        let fps_a = store.matching_fps("t", |l| l.get("svc") == Some("a"));
        let fps_b = store.matching_fps("t", |l| l.get("svc") == Some("b"));
        assert_eq!(fps_a.len(), 1);
        assert_eq!(fps_b.len(), 1);
        assert_ne!(fps_a, fps_b);
    }

    #[test]
    fn store_label_values_filtered_by_tenant() {
        let store = LogStore::new();
        store
            .push("t1", lbl(&[("env", "prod")]), vec![LogEntry::new(0, "l")])
            .unwrap();
        store
            .push("t2", lbl(&[("env", "dev")]), vec![LogEntry::new(0, "l")])
            .unwrap();
        let v_t1 = store.label_values("env", "t1");
        let v_t2 = store.label_values("env", "t2");
        assert_eq!(v_t1, vec!["prod"]);
        assert_eq!(v_t2, vec!["dev"]);
    }

    #[test]
    fn store_count_over_buckets_distributes_entries_in_time() {
        let store = LogStore::new();
        let step = 1_000_000_000i64;
        let entries = vec![
            LogEntry::new(0, "a"),
            LogEntry::new(step / 2, "b"),
            LogEntry::new(step + 100, "c"),
            LogEntry::new(2 * step + 100, "d"),
        ];
        store.push("t", lbl(&[("k", "v")]), entries).unwrap();
        let fps = store.matching_fps("t", |_| true);
        let buckets = store.count_over_buckets("t", &fps, 0, 3 * step, step);
        assert_eq!(buckets.len(), 3);
        let counts: Vec<f64> = buckets.iter().map(|(_, c)| *c).collect();
        assert_eq!(counts, vec![2.0, 1.0, 1.0]);
    }
}

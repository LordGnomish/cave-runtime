<<<<<<< HEAD
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/sharp-wiles
//! Log aggregation, search & alerting — replaces ELK Stack / Grafana Loki.
//!
//! Replaces: Elasticsearch + Logstash + Kibana / Grafana Loki
//! Upstream tracking: see cave-upstream for monitored features.

pub mod alerting;
pub mod ingestion;
pub mod models;
pub mod query;
pub mod routes;

use axum::Router;
use models::{LogAlert, LogDashboard, LogEntry, LogPipeline, LogStream};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Shared in-memory state for cave-logs.
pub struct LogsState {
    /// Ring-buffer of all ingested log entries (global, across all streams).
    pub entries: Mutex<VecDeque<LogEntry>>,
    pub streams: Mutex<HashMap<Uuid, LogStream>>,
    pub alerts: Mutex<HashMap<Uuid, LogAlert>>,
    pub pipelines: Mutex<HashMap<Uuid, LogPipeline>>,
    pub dashboards: Mutex<HashMap<Uuid, LogDashboard>>,
}

impl Default for LogsState {
    fn default() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            streams: Mutex::new(HashMap::new()),
            alerts: Mutex::new(HashMap::new()),
            pipelines: Mutex::new(HashMap::new()),
            dashboards: Mutex::new(HashMap::new()),
        }
    }
}

/// Create the axum router for this module.
<<<<<<< HEAD
=======
//! CAVE Logs — structured log ingestion and query engine.
//!
//! Replaces Loki with a Rust-native implementation.
//! Supports Loki push API, LogQL instant and range queries,
//! and label enumeration for Grafana/Alloy compatibility.
//!
//! ## Upstream Compatibility: Loki
//! - Push:          POST /loki/api/v1/push
//! - Instant query: GET  /loki/api/v1/query
//! - Range query:   GET  /loki/api/v1/query_range
//! - Labels:        GET  /loki/api/v1/labels
//! - Label values:  GET  /loki/api/v1/label/:name/values
//!
//! ## Upstream Tracking: Grafana Loki
//! - GitHub: https://github.com/grafana/loki
//! - Tracked: push API, LogQL query API, label API

pub mod models;
pub mod routes;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state shared across request handlers.
pub struct LogsState {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for the logs module.
>>>>>>> claude/gallant-cartwright
=======
//! CAVE Logs — production-grade log aggregation with full Loki/LogQL feature parity.
//!
//! ## Implemented upstream features
//!
//! | Feature | Loki upstream | Status |
//! |---------|--------------|--------|
//! | Push API (JSON) | `/loki/api/v1/push` | ✓ |
//! | Push API (Protobuf+Snappy) | `/loki/api/v1/push` | ✓ |
//! | LogQL stream selectors | `{app="foo"}` | ✓ |
//! | LogQL filter expressions | `\|= "str"`, `\|~ "re"` | ✓ |
//! | LogQL parsers | `\| json`, `\| logfmt`, `\| regexp`, `\| pattern`, `\| unpack` | ✓ |
//! | LogQL label filters | `\| status >= 400` | ✓ |
//! | LogQL line format | `\| line_format "{{.f}}"` | ✓ |
//! | Metric queries | `rate`, `count_over_time`, `bytes_over_time`, aggregations | ✓ |
//! | Instant query API | `/loki/api/v1/query` | ✓ |
//! | Range query API | `/loki/api/v1/query_range` | ✓ |
//! | Labels API | `/loki/api/v1/labels` | ✓ |
//! | Label values API | `/loki/api/v1/label/:name/values` | ✓ |
//! | Series API | `/loki/api/v1/series` | ✓ |
//! | WebSocket tail | `/loki/api/v1/tail` | ✓ |
//! | Multi-tenant (`X-Scope-OrgID`) | header isolation | ✓ |
//! | Structured metadata | per-entry key/value | ✓ |
//! | Log-based alerting | rule evaluation loop | ✓ |
//! | Chunk retention | configurable TTL + pruning | ✓ |

pub mod alerting;
pub mod logql;
pub mod models;
pub mod push;
pub mod routes;
pub mod store;
pub mod tail;

use axum::Router;
use chrono::Duration;
use std::sync::Arc;

pub use alerting::AlertManager;
pub use store::LogStore;

/// Shared module state — injected into every route handler.
pub struct LogsState {
    pub store: Arc<LogStore>,
    pub alert_manager: Arc<AlertManager>,
    pub default_limit: usize,
}

impl LogsState {
    pub fn new(retention_days: i64, default_limit: usize) -> Arc<Self> {
        let store = Arc::new(LogStore::new(Duration::days(retention_days)));
        let alert_manager = Arc::new(AlertManager::new(Arc::clone(&store)));
        Arc::new(Self { store, alert_manager, default_limit })
    }
}

/// Build the axum router for the logs module.
///
/// Mount at the root or under a prefix — Loki clients use `/loki/api/v1/*`.
>>>>>>> claude/inspiring-pascal
=======
>>>>>>> claude/sharp-wiles
pub fn router(state: Arc<LogsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "logs";
<<<<<<< HEAD
<<<<<<< HEAD
=======

// ─── Integration tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Labels, LogEntry, PushRequest, StreamPush};
    use crate::push::{ingest_json, ingest_proto};
    use crate::store::LogStore;
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn make_store() -> Arc<LogStore> {
        Arc::new(LogStore::new(Duration::days(7)))
    }

    fn sample_labels(app: &str) -> Labels {
        Labels::new([("app".into(), app.into())].into())
    }

    fn make_entry(line: &str, offset_secs: i64) -> LogEntry {
        LogEntry {
            timestamp: Utc::now() - Duration::seconds(offset_secs),
            line: line.into(),
            structured_metadata: HashMap::new(),
        }
    }

    // ── Push: JSON format ─────────────────────────────────────────────────────

    #[test]
    fn push_json_single_stream() {
        let store = make_store();
        let ts = Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string();
        let req = PushRequest {
            streams: vec![StreamPush {
                stream: [("app".into(), "test".into())].into(),
                values: vec![serde_json::json!([ts, "hello world"])],
            }],
        };
        ingest_json(&store, req, None);
        assert_eq!(store.stream_count(), 1);
        assert_eq!(store.entry_count(), 1);
    }

    #[test]
    fn push_json_multi_stream() {
        let store = make_store();
        let ts = Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string();
        let req = PushRequest {
            streams: vec![
                StreamPush {
                    stream: [("app".into(), "svc-a".into())].into(),
                    values: vec![serde_json::json!([ts, "msg from a"])],
                },
                StreamPush {
                    stream: [("app".into(), "svc-b".into())].into(),
                    values: vec![serde_json::json!([ts, "msg from b"])],
                },
            ],
        };
        ingest_json(&store, req, None);
        assert_eq!(store.stream_count(), 2);
    }

    #[test]
    fn push_json_with_structured_metadata() {
        let store = make_store();
        let ts = Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string();
        let req = PushRequest {
            streams: vec![StreamPush {
                stream: [("job".into(), "meta-test".into())].into(),
                values: vec![serde_json::json!([
                    ts,
                    "line with metadata",
                    {"trace_id": "abc123", "span_id": "def456"}
                ])],
            }],
        };
        ingest_json(&store, req, None);
        assert_eq!(store.entry_count(), 1);
    }

    // ── Push: Protobuf format ─────────────────────────────────────────────────

    #[test]
    fn push_proto_roundtrip() {
        use crate::models::proto::{EntryAdapter, PushRequest as ProtoPush, StreamAdapter};
        use prost::Message;

        let ts_proto = prost_types::Timestamp {
            seconds: Utc::now().timestamp(),
            nanos: 0,
        };
        let req = ProtoPush {
            streams: vec![StreamAdapter {
                labels: r#"{app="proto-test"}"#.into(),
                entries: vec![EntryAdapter {
                    timestamp: Some(ts_proto),
                    line: "proto log line".into(),
                    structured_metadata: vec![],
                }],
                hash: String::new(),
            }],
        };

        // Encode protobuf
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();

        // Compress with snappy
        let mut enc = snap::raw::Encoder::new();
        let compressed = enc.compress_vec(&buf).unwrap();

        let store = make_store();
        ingest_proto(&store, bytes::Bytes::from(compressed), None).unwrap();
        assert_eq!(store.stream_count(), 1);
        assert_eq!(store.entry_count(), 1);
    }

    // ── Multi-tenant isolation ────────────────────────────────────────────────

    #[test]
    fn multi_tenant_isolation() {
        let store = make_store();
        let labels = sample_labels("shared-name");

        store.push(labels.clone(), vec![make_entry("tenant-a log", 10)], Some("tenant-a".into()));
        store.push(labels.clone(), vec![make_entry("tenant-b log", 10)], Some("tenant-b".into()));

        let now = Utc::now();
        let start = now - Duration::hours(1);
        let use_matchers: Vec<crate::models::LabelMatcher> = vec![];

        let a_streams =
            store.query_streams(&use_matchers, start, now, 100, true, Some("tenant-a"));
        let b_streams =
            store.query_streams(&use_matchers, start, now, 100, true, Some("tenant-b"));

        assert_eq!(a_streams.len(), 1);
        assert!(a_streams[0].1[0].line.contains("tenant-a"));
        assert_eq!(b_streams.len(), 1);
        assert!(b_streams[0].1[0].line.contains("tenant-b"));
    }

    // ── Label queries ─────────────────────────────────────────────────────────

    #[test]
    fn label_names_query() {
        let store = make_store();
        store.push(
            Labels::new([("app".into(), "x".into()), ("env".into(), "prod".into())].into()),
            vec![make_entry("line", 5)],
            None,
        );

        let now = Utc::now();
        let names = store.label_names(now - Duration::hours(1), now, None);
        assert!(names.contains(&"app".into()));
        assert!(names.contains(&"env".into()));
    }

    #[test]
    fn label_values_query() {
        let store = make_store();
        store.push(
            Labels::new([("env".into(), "prod".into())].into()),
            vec![make_entry("l1", 5)],
            None,
        );
        store.push(
            Labels::new([("env".into(), "staging".into())].into()),
            vec![make_entry("l2", 5)],
            None,
        );

        let now = Utc::now();
        let values = store.label_values("env", now - Duration::hours(1), now, None);
        assert!(values.contains(&"prod".into()));
        assert!(values.contains(&"staging".into()));
    }

    // ── Stream selection ──────────────────────────────────────────────────────

    #[test]
    fn stream_selection_exact() {
        let store = make_store();
        store.push(sample_labels("foo"), vec![make_entry("foo line", 5)], None);
        store.push(sample_labels("bar"), vec![make_entry("bar line", 5)], None);

        let now = Utc::now();
        let matchers = logql::parser::parse(r#"{app="foo"}"#)
            .map(|e| match e { crate::logql::ast::Expr::Log(ls) => ls.matchers, _ => vec![] })
            .unwrap();
        let results = store.query_streams(&matchers, now - Duration::hours(1), now, 100, true, None);
        assert_eq!(results.len(), 1);
        assert!(results[0].1[0].line.contains("foo"));
    }

    #[test]
    fn stream_selection_regex() {
        let store = make_store();
        store.push(
            Labels::new([("env".into(), "production".into())].into()),
            vec![make_entry("prod log", 5)],
            None,
        );
        store.push(
            Labels::new([("env".into(), "staging".into())].into()),
            vec![make_entry("staging log", 5)],
            None,
        );
        store.push(
            Labels::new([("env".into(), "dev".into())].into()),
            vec![make_entry("dev log", 5)],
            None,
        );

        let now = Utc::now();
        let matchers = logql::parser::parse(r#"{env=~"prod.*|staging"}"#)
            .map(|e| match e { crate::logql::ast::Expr::Log(ls) => ls.matchers, _ => vec![] })
            .unwrap();
        let results = store.query_streams(&matchers, now - Duration::hours(1), now, 100, true, None);
        assert_eq!(results.len(), 2, "should match production and staging");
    }

    // ── Retention ─────────────────────────────────────────────────────────────

    #[test]
    fn retention_prunes_old_entries() {
        let store = Arc::new(LogStore::new(Duration::seconds(1)));
        store.push(
            sample_labels("prune-test"),
            vec![LogEntry {
                // Entry from 2 seconds ago, older than 1-second retention
                timestamp: Utc::now() - Duration::seconds(2),
                line: "old log".into(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );

        assert_eq!(store.entry_count(), 1);
        store.prune();
        assert_eq!(store.entry_count(), 0, "old entry should be pruned");
    }

    // ── Series API ────────────────────────────────────────────────────────────

    #[test]
    fn series_returns_matching_label_sets() {
        let store = make_store();
        for app in ["alpha", "beta", "gamma"] {
            store.push(
                Labels::new([("app".into(), app.into()), ("ns".into(), "default".into())].into()),
                vec![make_entry("line", 5)],
                None,
            );
        }

        let matchers = logql::parser::parse(r#"{ns="default"}"#)
            .map(|e| match e { crate::logql::ast::Expr::Log(ls) => ls.matchers, _ => vec![] })
            .unwrap();
        let now = Utc::now();
        let series = store.series(&matchers, now - Duration::hours(1), now, None);
        assert_eq!(series.len(), 3);
    }

    // ── Logfmt parser ─────────────────────────────────────────────────────────

    #[test]
    fn logfmt_extracts_fields() {
        let mut entry = crate::logql::eval::ProcessedEntry {
            timestamp: Utc::now(),
            line: r#"level=info method=GET status=200 path="/health""#.into(),
            extracted: HashMap::new(),
        };
        // Call the logfmt parser directly (via module internals)
        crate::logql::eval::parse_logfmt_pub(&mut entry);
        assert_eq!(entry.extracted.get("level"), Some(&"info".into()));
        assert_eq!(entry.extracted.get("method"), Some(&"GET".into()));
        assert_eq!(entry.extracted.get("status"), Some(&"200".into()));
    }
}
>>>>>>> claude/inspiring-pascal
=======
>>>>>>> claude/sharp-wiles

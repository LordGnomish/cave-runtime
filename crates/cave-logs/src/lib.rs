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
pub mod limits;
pub mod logql;
pub mod models;
pub mod multitenant;
pub mod routes;
pub mod store;
pub mod tail;

pub use routes::{router, AppState};
pub use store::LogStore;
pub use limits::LimitsRegistry;

/// Create a fully initialised `AppState` with default configuration.
pub fn default_state() -> AppState {
    AppState {
        store: LogStore::new(),
        limits: LimitsRegistry::with_defaults(),
    }
}

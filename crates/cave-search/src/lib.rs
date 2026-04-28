//! cave-search — OpenSearch / Elasticsearch wire-faithful search engine.
//!
//! Surfaces:
//!
//! * `mapping` — field type definitions (text/keyword/long/double/date/boolean/object).
//! * `index` — per-index document store + inverted index over keyword fields.
//! * `dsl` — JSON Query DSL: term/match/range/bool/match_all/exists/prefix.
//! * `aggregation` — terms / sum / avg / min / max / value_count / cardinality.
//! * `server` — HTTP API mirrored from OpenSearch:
//!   PUT /:index, DELETE /:index, GET /:index, POST /:index/_doc,
//!   GET /:index/_doc/:id, POST /:index/_search, POST /:index/_bulk,
//!   GET /_cluster/health, GET /_cat/indices

pub mod aggregation;
pub mod cluster;
pub mod dsl;
pub mod error;
pub mod index;
pub mod mapping;
pub mod server;

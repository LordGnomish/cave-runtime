//! Prometheus HTTP API v1 — full parity implementation.

pub mod query;
pub mod labels;
pub mod series;
pub mod metadata;
pub mod targets;
pub mod rules;
pub mod remote;
pub mod status;
pub mod federation;

use axum::{routing::{get, post}, Router};
use std::sync::Arc;
use crate::state::MetricsState;

pub fn create_router(state: Arc<MetricsState>) -> Router {
    Router::new()
        // ── Query ──────────────────────────────────────────────────────────
        .route("/api/v1/query",          get(query::instant_query).post(query::instant_query))
        .route("/api/v1/query_range",    get(query::range_query).post(query::range_query))
        .route("/api/v1/query_exemplars", get(query::exemplars))

        // ── Metadata ──────────────────────────────────────────────────────
        .route("/api/v1/labels",                    get(labels::list_labels).post(labels::list_labels))
        .route("/api/v1/label/:name/values",        get(labels::label_values))
        .route("/api/v1/series",                    get(series::list_series).post(series::list_series))
        .route("/api/v1/metadata",                  get(metadata::metric_metadata))

        // ── Targets ───────────────────────────────────────────────────────
        .route("/api/v1/targets",                   get(targets::list_targets))
        .route("/api/v1/targets/metadata",          get(targets::targets_metadata))

        // ── Rules / Alerts ────────────────────────────────────────────────
        .route("/api/v1/rules",                     get(rules::list_rules))
        .route("/api/v1/alerts",                    get(rules::list_alerts))

        // ── Remote write / read ───────────────────────────────────────────
        .route("/api/v1/write",                     post(remote::remote_write))
        .route("/api/v1/read",                      post(remote::remote_read))

        // ── Status ────────────────────────────────────────────────────────
        .route("/api/v1/status/config",             get(status::config))
        .route("/api/v1/status/flags",              get(status::flags))
        .route("/api/v1/status/runtimeinfo",        get(status::runtime_info))
        .route("/api/v1/status/buildinfo",          get(status::build_info))
        .route("/api/v1/status/tsdb",               get(status::tsdb_stats))
        .route("/api/v1/status/walreplay",          get(status::wal_replay))

        // ── Federation ────────────────────────────────────────────────────
        .route("/federate",                         get(federation::federate))

        // ── Ingestion shortcuts ───────────────────────────────────────────
        .route("/api/v1/otlp/v1/metrics",           post(remote::otlp_metrics))
        .route("/api/v1/statsd",                    post(remote::statsd_ingest))
        .route("/api/v1/graphite",                  post(remote::graphite_ingest))
        .route("/api/v1/influx/write",              post(remote::influx_write))
        .route("/api/v1/influx/api/v2/write",       post(remote::influx_write))

        .with_state(state)
}

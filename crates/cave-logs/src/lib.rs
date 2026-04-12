<<<<<<< HEAD
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
pub fn router(state: Arc<LogsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "logs";

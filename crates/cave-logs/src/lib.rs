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
pub fn router(state: Arc<LogsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "logs";

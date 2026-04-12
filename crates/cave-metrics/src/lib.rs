//! Metrics collection and querying — Prometheus/Thanos replacement.
//!
//! Replaces: Prometheus, Thanos, Alertmanager
//! PromQL-like querying, scrape targets, alert/recording rules, time series storage.

pub mod alerting;
pub mod models;
pub mod query;
pub mod routes;
pub mod scraper;
pub mod storage;

use axum::Router;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Shared state for cave-metrics.
pub struct MetricsState {
    pub store: Arc<Mutex<storage::TimeSeriesStore>>,
    pub alert_rules: Arc<Mutex<Vec<models::AlertRule>>>,
    pub recording_rules: Arc<Mutex<Vec<models::RecordingRule>>>,
    pub scrape_targets: Arc<Mutex<Vec<models::ScrapeTarget>>>,
    pub metadata: Arc<Mutex<Vec<models::MetricMetadata>>>,
}

impl MetricsState {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(storage::TimeSeriesStore::default())),
            alert_rules: Arc::new(Mutex::new(Vec::new())),
            recording_rules: Arc::new(Mutex::new(Vec::new())),
            scrape_targets: Arc::new(Mutex::new(Vec::new())),
            metadata: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Default for MetricsState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<MetricsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "metrics";

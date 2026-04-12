//! MetricsState: the shared application state.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use axum::Router;
use axum::routing::{get, post};
use crate::alertmanager::{AlertmanagerClient, AlertmanagerConfig};
use crate::promql::Engine;
use crate::rules::{AlertingRule, RecordingRule};
use crate::scrape::ScrapeManager;
use crate::tsdb::{Tsdb, TsdbConfig};

pub struct MetricsConfig {
    pub tsdb: TsdbConfig,
    pub alertmanager: Option<AlertmanagerConfig>,
    pub scrape_interval_ms: u64,
    pub rules_interval_ms: u64,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            tsdb: TsdbConfig::default(),
            alertmanager: None,
            scrape_interval_ms: 15_000,
            rules_interval_ms: 15_000,
        }
    }
}

pub struct MetricsState {
    pub tsdb: Arc<Tsdb>,
    pub engine: Arc<Engine>,
    pub scrape_manager: Arc<ScrapeManager>,
    pub recording_rules: Arc<RwLock<Vec<RecordingRule>>>,
    pub alerting_rules: Arc<RwLock<Vec<AlertingRule>>>,
    pub alertmanager: Option<Arc<AlertmanagerClient>>,
    pub config: MetricsConfig,
}

impl MetricsState {
    pub fn new(config: MetricsConfig) -> Self {
        let tsdb = Arc::new(Tsdb::new(config.tsdb.clone()).expect("Failed to initialize TSDB"));
        let engine = Arc::new(Engine::new());
        let scrape_manager = Arc::new(ScrapeManager::new());
        let alertmanager = config.alertmanager.as_ref().map(|cfg| {
            Arc::new(AlertmanagerClient::new(cfg.clone()))
        });
        Self {
            tsdb,
            engine,
            scrape_manager,
            recording_rules: Arc::new(RwLock::new(Vec::new())),
            alerting_rules: Arc::new(RwLock::new(Vec::new())),
            alertmanager,
            config,
        }
    }

    pub fn router(self: Arc<Self>) -> Router {
        use crate::api::{query, labels, series, remote_write};
        Router::new()
            .route("/api/v1/query", get(query::instant_query))
            .route("/api/v1/query_range", get(query::range_query))
            .route("/api/v1/series", get(series::series))
            .route("/api/v1/labels", get(labels::label_names))
            .route("/api/v1/label/{name}/values", get(labels::label_values))
            .route("/api/v1/write", post(remote_write::remote_write))
            .with_state(self)
    }

    pub async fn start(self: Arc<Self>) {
        // Start retention background task
        self.tsdb.clone().start_retention_task();

        // Start scraping
        self.scrape_manager.clone().start(self.tsdb.clone());

        // Start rules evaluation
        let state = self.clone();
        let rules_interval_ms = self.config.rules_interval_ms;
        tokio::spawn(async move {
            let interval = std::time::Duration::from_millis(rules_interval_ms);
            let mut pending_alerts: HashMap<String, HashMap<String, i64>> = HashMap::new();
            loop {
                tokio::time::sleep(interval).await;
                let now_ms = chrono::Utc::now().timestamp_millis();

                // Evaluate recording rules
                let recording = state.recording_rules.read().clone();
                for rule in &recording {
                    if let Err(e) = rule.evaluate(&state.engine, &state.tsdb, now_ms).await {
                        tracing::warn!("Recording rule '{}' failed: {}", rule.name, e);
                    }
                }

                // Evaluate alerting rules
                let alerting = state.alerting_rules.read().clone();
                let mut all_alerts = Vec::new();
                for rule in &alerting {
                    let pending = pending_alerts.entry(rule.name.clone()).or_default();
                    match rule.evaluate(&state.engine, &state.tsdb, now_ms, pending).await {
                        Ok(alerts) => all_alerts.extend(alerts),
                        Err(e) => tracing::warn!("Alerting rule '{}' failed: {}", rule.name, e),
                    }
                }

                // Send firing alerts to alertmanager
                if let Some(am) = &state.alertmanager {
                    let firing: Vec<_> = all_alerts.iter()
                        .filter(|a| a.state == crate::rules::AlertState::Firing)
                        .cloned()
                        .collect();
                    if !firing.is_empty() {
                        if let Err(e) = am.send_alerts(&firing).await {
                            tracing::warn!("Failed to send alerts to alertmanager: {}", e);
                        }
                    }
                }
            }
        });
    }
}

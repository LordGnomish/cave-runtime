// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scrape manager: discovers, scrapes, and injects metrics into the TSDB.

use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::ingestion::exposition;
use crate::ingestion::openmetrics;
use crate::model::{Labels, Sample};
use crate::tsdb::Tsdb;
use super::target::{ScrapeConfig, ScrapeTarget};
use super::discovery;

pub struct ScrapeManager {
    configs: Arc<RwLock<Vec<ScrapeConfig>>>,
    targets: Arc<RwLock<Vec<ScrapeTarget>>>,
    tsdb: Arc<Tsdb>,
}

impl ScrapeManager {
    pub fn new(tsdb: Arc<Tsdb>) -> Self {
        Self {
            configs: Arc::new(RwLock::new(Vec::new())),
            targets: Arc::new(RwLock::new(Vec::new())),
            tsdb,
        }
    }

    pub fn add_config(&self, config: ScrapeConfig) {
        let new_targets = discovery::resolve_all(&config);
        self.configs.write().push(config);
        self.targets.write().extend(new_targets);
    }

    pub fn remove_config(&self, job_name: &str) {
        self.configs.write().retain(|c| c.job_name != job_name);
        self.targets.write().retain(|t| t.config.job_name != job_name);
    }

    pub fn targets(&self) -> Vec<ScrapeTarget> {
        self.targets.read().clone()
    }

    /// Perform one scrape cycle: iterate targets due for scraping.
    pub async fn scrape_cycle(&self) {
        let now_ms = now_ms();
        let targets_snap: Vec<(usize, ScrapeTarget)> = self.targets.read().iter().enumerate()
            .filter(|(_, t)| now_ms - t.last_scrape_ms >= t.config.scrape_interval_ms)
            .map(|(i, t)| (i, t.clone()))
            .collect();

        for (idx, target) in targets_snap {
            let result = self.scrape_target(&target).await;
            let mut targets = self.targets.write();
            if let Some(t) = targets.get_mut(idx) {
                t.last_scrape_ms = now_ms;
                match result {
                    Ok(n) => {
                        t.last_error = None;
                        t.last_duration_ms = now_ms - t.last_scrape_ms;
                        info!("Scraped {} — {} series", t.url, n);
                    }
                    Err(e) => {
                        t.last_error = Some(e.to_string());
                        warn!("Scrape {} failed: {}", t.url, e);
                    }
                }
            }
        }
    }

    async fn scrape_target(&self, target: &ScrapeTarget) -> crate::error::Result<usize> {
        let timeout = Duration::from_millis(target.config.scrape_timeout_ms as u64);

        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();

        let resp = client.get(&target.url)
            .send()
            .await
            .map_err(|e| crate::error::MetricsError::Scrape(e.to_string()))?;

        let content_type = resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = resp.text().await
            .map_err(|e| crate::error::MetricsError::Scrape(e.to_string()))?;

        let batch = if openmetrics::is_openmetrics(&content_type) {
            openmetrics::parse(&body)?
        } else {
            exposition::parse(&body)?
        };

        let n = batch.len();
        let ts_ms = now_ms();

        for mut ts in batch {
            // Apply target labels (honor_labels controls precedence)
            if target.config.honor_labels {
                // Target labels only fill gaps
                for (k, v) in target.labels.iter() {
                    if ts.labels.get(k).is_none() {
                        ts.labels.insert(k, v);
                    }
                }
            } else {
                // Target labels overwrite
                for (k, v) in target.labels.iter() {
                    ts.labels.insert(k, v);
                }
            }

            for mut sample in ts.samples {
                if !target.config.honor_timestamps {
                    sample.timestamp_ms = ts_ms;
                }
                self.tsdb.append(ts.labels.clone(), sample);
            }
        }

        Ok(n)
    }

    /// Spawn a background task that continuously scrapes.
    pub fn start(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut tick = interval(Duration::from_secs(5));
            loop {
                tick.tick().await;
                self.scrape_cycle().await;
            }
        });
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

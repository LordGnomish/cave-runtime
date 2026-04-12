//! Scrape target management and execution.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use crate::error::MetricsResult;
use crate::exposition::parse_exposition;
use crate::model::{Labels, Timestamp, Value};
use crate::tsdb::Tsdb;

#[derive(Debug, Clone)]
pub struct ScrapeTarget {
    pub id: String,
    pub url: String,
    pub interval_ms: u64,
    pub labels: Labels,
    pub honor_labels: bool,
}

pub struct ScrapeManager {
    targets: Arc<RwLock<HashMap<String, ScrapeTarget>>>,
    client: reqwest::Client,
}

impl ScrapeManager {
    pub fn new() -> Self {
        Self {
            targets: Arc::new(RwLock::new(HashMap::new())),
            client: reqwest::Client::new(),
        }
    }

    pub fn add_target(&self, target: ScrapeTarget) {
        self.targets.write().insert(target.id.clone(), target);
    }

    pub fn remove_target(&self, id: &str) {
        self.targets.write().remove(id);
    }

    pub fn list_targets(&self) -> Vec<ScrapeTarget> {
        self.targets.read().values().cloned().collect()
    }

    pub async fn scrape(
        &self,
        target: &ScrapeTarget,
    ) -> MetricsResult<Vec<(Labels, Value, Option<Timestamp>)>> {
        let body = self.client
            .get(&target.url)
            .send()
            .await?
            .text()
            .await?;
        let mut samples = parse_exposition(&body)?;
        // Apply target labels
        for (labels, _, _) in &mut samples {
            for (k, v) in &target.labels.0 {
                if target.honor_labels {
                    labels.0.entry(k.clone()).or_insert_with(|| v.clone());
                } else {
                    if labels.0.contains_key(k) {
                        // rename conflicting label with exported_ prefix
                        let exported_key = format!("exported_{}", k);
                        let existing = labels.0.remove(k).unwrap();
                        labels.0.insert(exported_key, existing);
                    }
                    labels.0.insert(k.clone(), v.clone());
                }
            }
        }
        Ok(samples)
    }

    pub fn start(self: Arc<Self>, tsdb: Arc<Tsdb>) {
        let targets: Vec<ScrapeTarget> = self.targets.read().values().cloned().collect();
        for target in targets {
            let mgr = self.clone();
            let tsdb = tsdb.clone();
            let interval = std::time::Duration::from_millis(target.interval_ms);
            tokio::spawn(async move {
                loop {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    match mgr.scrape(&target).await {
                        Ok(samples) => {
                            for (labels, value, ts) in samples {
                                let timestamp = ts.unwrap_or(now_ms);
                                if let Err(e) = tsdb.append(labels, timestamp, value) {
                                    tracing::warn!("Failed to append scraped sample: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Scrape failed for {}: {}", target.url, e);
                        }
                    }
                    tokio::time::sleep(interval).await;
                }
            });
        }
    }
}

impl Default for ScrapeManager {
    fn default() -> Self {
        Self::new()
    }
}

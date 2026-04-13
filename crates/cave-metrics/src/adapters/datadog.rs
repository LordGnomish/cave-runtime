//! Datadog Metrics API adapter.
//!
//! Forwards metrics to Datadog via the v2 Metrics API.
//!
//! # Configuration
//!
//! ```toml
//! [metrics]
//! backend    = "datadog"
//! dd_api_key = "..."
//! dd_site    = "datadoghq.com"   # or datadoghq.eu
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::{MetricsBackend, MetricsBackendError, MetricsResult};
use crate::model::{Labels, Sample, TimeSeries};

#[derive(Debug, Clone, Deserialize)]
pub struct DatadogConfig {
    pub api_key: String,
    /// Datadog site, e.g. `datadoghq.com` or `datadoghq.eu`.
    pub site: String,
}

impl DatadogConfig {
    pub fn metrics_url(&self) -> String {
        format!("https://api.{}/api/v2/series", self.site)
    }

    pub fn query_url(&self) -> String {
        format!("https://api.{}/api/v1/query", self.site)
    }
}

/// Convert Labels to Datadog tags `["key:value", ...]`.
fn labels_to_dd_tags(labels: &Labels) -> Vec<String> {
    labels
        .0
        .iter()
        .filter(|(k, _)| k.as_str() != "__name__")
        .map(|(k, v)| format!("{}:{}", k, v))
        .collect()
}

/// Extract metric name from `__name__` label.
fn metric_name(labels: &Labels) -> String {
    labels
        .get("__name__")
        .unwrap_or("unknown")
        .replace('_', ".")   // Datadog convention: dots, not underscores
        .to_string()
}

/// Datadog v2 series payload.
#[derive(Serialize)]
struct DdSeries<'a> {
    series: Vec<DdMetric<'a>>,
}

#[derive(Serialize)]
struct DdMetric<'a> {
    metric: String,
    /// 0=unspecified, 1=count, 2=rate, 3=gauge
    #[serde(rename = "type")]
    metric_type: u8,
    points: Vec<DdPoint>,
    tags: Vec<String>,
    resources: Vec<DdResource<'a>>,
}

#[derive(Serialize)]
struct DdPoint {
    timestamp: i64,
    value: f64,
}

#[derive(Serialize)]
struct DdResource<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    res_type: &'a str,
}

/// Datadog Metrics API adapter.
pub struct DatadogAdapter {
    config: DatadogConfig,
    client: reqwest::Client,
}

impl DatadogAdapter {
    pub fn new(config: DatadogConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    /// Send a batch of at most `chunk` time-series to Datadog.
    async fn send_chunk(&self, chunk: &[TimeSeries]) -> MetricsResult<()> {
        let series: Vec<DdMetric> = chunk
            .iter()
            .map(|ts| {
                let points = ts
                    .samples
                    .iter()
                    .map(|s| DdPoint {
                        timestamp: s.timestamp_ms / 1000, // DD wants seconds
                        value: s.value,
                    })
                    .collect();
                DdMetric {
                    metric: metric_name(&ts.labels),
                    metric_type: 3, // gauge
                    points,
                    tags: labels_to_dd_tags(&ts.labels),
                    resources: vec![],
                }
            })
            .collect();

        let payload = DdSeries { series };

        let resp = self
            .client
            .post(self.config.metrics_url())
            .header("DD-API-KEY", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| MetricsBackendError::Unreachable(format!("Datadog request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MetricsBackendError::WriteFailed(format!(
                "Datadog returned {status}: {body}"
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl MetricsBackend for DatadogAdapter {
    async fn write(&self, batch: Vec<TimeSeries>) -> MetricsResult<()> {
        // Datadog v2 series API accepts up to 500KB per request; we chunk by
        // 500 series which is well within that limit for typical cardinality.
        for chunk in batch.chunks(500) {
            self.send_chunk(chunk).await?;
        }
        Ok(())
    }

    async fn query_instant(&self, expr: &str, timestamp_ms: i64) -> MetricsResult<serde_json::Value> {
        // Datadog v1 /query uses DogStatsD-style metric names, not PromQL.
        // We pass the expression through as-is and let Datadog interpret it.
        let from_sec = timestamp_ms / 1000 - 60;
        let to_sec = timestamp_ms / 1000;

        let resp = self
            .client
            .get(self.config.query_url())
            .header("DD-API-KEY", &self.config.api_key)
            .header("DD-APPLICATION-KEY", std::env::var("DD_APP_KEY").unwrap_or_default())
            .query(&[
                ("query", expr),
                ("from", &from_sec.to_string()),
                ("to", &to_sec.to_string()),
            ])
            .send()
            .await
            .map_err(|e| MetricsBackendError::QueryFailed(format!("Datadog query failed: {e}")))?;

        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| MetricsBackendError::QueryFailed(format!("Datadog query parse error: {e}")))
    }

    async fn query_range(
        &self,
        expr: &str,
        start_ms: i64,
        end_ms: i64,
        _step_ms: i64,
    ) -> MetricsResult<serde_json::Value> {
        let resp = self
            .client
            .get(self.config.query_url())
            .header("DD-API-KEY", &self.config.api_key)
            .header("DD-APPLICATION-KEY", std::env::var("DD_APP_KEY").unwrap_or_default())
            .query(&[
                ("query", expr),
                ("from", &(start_ms / 1000).to_string()),
                ("to", &(end_ms / 1000).to_string()),
            ])
            .send()
            .await
            .map_err(|e| MetricsBackendError::QueryFailed(format!("Datadog range query failed: {e}")))?;

        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| MetricsBackendError::QueryFailed(format!("Datadog range query parse error: {e}")))
    }

    async fn label_names(&self) -> MetricsResult<Vec<String>> {
        // Fetch active metric names from Datadog.
        let url = format!("https://api.{}/api/v1/metrics", self.config.site);
        let from_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(3600);

        let resp = self
            .client
            .get(&url)
            .header("DD-API-KEY", &self.config.api_key)
            .header("DD-APPLICATION-KEY", std::env::var("DD_APP_KEY").unwrap_or_default())
            .query(&[("from", from_sec.to_string())])
            .send()
            .await
            .map_err(|_| MetricsBackendError::Unreachable("Datadog metrics list failed".into()))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        #[derive(Deserialize)]
        struct MetricsList {
            metrics: Vec<String>,
        }

        let list: MetricsList = resp.json().await.unwrap_or(MetricsList { metrics: vec![] });
        Ok(list.metrics)
    }

    fn name(&self) -> &'static str {
        "datadog"
    }
}

//! New Relic Metric API adapter.
//!
//! Forwards metrics to New Relic via the Metric API (dimensional metrics).
//!
//! # Configuration
//!
//! ```toml
//! [metrics]
//! backend        = "new_relic"
//! nr_license_key = "..."
//! nr_region      = "US"   # or "EU"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::backend::{MetricsBackend, MetricsBackendError, MetricsResult};
use crate::model::TimeSeries;

#[derive(Debug, Clone, Deserialize)]
pub struct NewRelicConfig {
    pub license_key: String,
    /// NR account ID (required for NerdGraph queries).
    pub account_id: String,
    /// `"US"` or `"EU"`.
    pub region: String,
}

impl NewRelicConfig {
    pub fn metric_api_url(&self) -> &'static str {
        match self.region.as_str() {
            "EU" => "https://metric-api.eu.newrelic.com/metric/v1",
            _ => "https://metric-api.newrelic.com/metric/v1",
        }
    }

    pub fn nerdgraph_url(&self) -> &'static str {
        match self.region.as_str() {
            "EU" => "https://api.eu.newrelic.com/graphql",
            _ => "https://api.newrelic.com/graphql",
        }
    }
}

/// New Relic dimensional metric payload root.
#[derive(Serialize)]
struct NrPayload {
    metrics: Vec<NrMetric>,
}

#[derive(Serialize)]
struct NrMetric {
    name: String,
    /// "gauge", "count", "summary"
    #[serde(rename = "type")]
    metric_type: String,
    value: f64,
    timestamp: i64,
    attributes: std::collections::HashMap<String, String>,
}

/// New Relic Metric API adapter.
pub struct NewRelicAdapter {
    config: NewRelicConfig,
    client: reqwest::Client,
}

impl NewRelicAdapter {
    pub fn new(config: NewRelicConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    async fn send_batch(&self, batch: &[TimeSeries]) -> MetricsResult<()> {
        let mut metrics: Vec<NrMetric> = Vec::with_capacity(batch.len() * 4);

        for ts in batch {
            let name = ts
                .labels
                .get("__name__")
                .unwrap_or("unknown")
                .to_string();

            let attributes: std::collections::HashMap<String, String> = ts
                .labels
                .0
                .iter()
                .filter(|(k, _)| k.as_str() != "__name__")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            for sample in &ts.samples {
                metrics.push(NrMetric {
                    name: name.clone(),
                    metric_type: "gauge".into(),
                    value: sample.value,
                    timestamp: sample.timestamp_ms / 1000,
                    attributes: attributes.clone(),
                });
            }
        }

        // NR Metric API accepts up to 1MB or 10,000 metrics per POST.
        for chunk in metrics.chunks(5000) {
            let payload = serde_json::json!([{"metrics": chunk}]);

            let resp = self
                .client
                .post(self.config.metric_api_url())
                .header("Api-Key", &self.config.license_key)
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| MetricsBackendError::Unreachable(format!("New Relic request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(MetricsBackendError::WriteFailed(format!(
                    "New Relic returned {status}: {body}"
                )));
            }
        }

        Ok(())
    }

    /// Execute a NRQL query via NerdGraph GraphQL.
    async fn nrql_query(&self, nrql: &str) -> MetricsResult<serde_json::Value> {
        let account_id = std::env::var("NR_ACCOUNT_ID").unwrap_or_default();
        let query = format!(
            r#"{{ actor {{ account(id: {account_id}) {{ nrql(query: "{nrql}") {{ results }} }} }} }}"#,
            account_id = account_id,
            nrql = nrql.replace('"', "\\\""),
        );

        let resp = self
            .client
            .post(self.config.nerdgraph_url())
            .header("Api-Key", &self.config.license_key)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"query": query}))
            .send()
            .await
            .map_err(|e| MetricsBackendError::QueryFailed(format!("NerdGraph request failed: {e}")))?;

        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| MetricsBackendError::QueryFailed(format!("NerdGraph parse error: {e}")))
    }
}

#[async_trait]
impl MetricsBackend for NewRelicAdapter {
    async fn write(&self, batch: Vec<TimeSeries>) -> MetricsResult<()> {
        self.send_batch(&batch).await
    }

    async fn query_instant(&self, expr: &str, timestamp_ms: i64) -> MetricsResult<serde_json::Value> {
        // Map a simple metric name to NRQL SELECT.
        // Full PromQL→NRQL translation is beyond scope; pass through as metric name.
        let since_secs = (chrono::Utc::now().timestamp_millis() - timestamp_ms) / 1000;
        let nrql = format!(
            "SELECT latest({}) FROM Metric SINCE {} seconds ago LIMIT 1",
            expr, since_secs.max(60)
        );
        self.nrql_query(&nrql).await
    }

    async fn query_range(
        &self,
        expr: &str,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> MetricsResult<serde_json::Value> {
        let duration_secs = (end_ms - start_ms) / 1000;
        let since_secs = (chrono::Utc::now().timestamp_millis() - start_ms) / 1000;
        let timeseries_secs = step_ms / 1000;
        let nrql = format!(
            "SELECT average({metric}) FROM Metric SINCE {since} seconds ago UNTIL {until} seconds ago TIMESERIES {step} seconds",
            metric = expr,
            since = since_secs.max(60),
            until = (since_secs - duration_secs).max(0),
            step = timeseries_secs.max(1),
        );
        self.nrql_query(&nrql).await
    }

    async fn label_names(&self) -> MetricsResult<Vec<String>> {
        let nrql = "SELECT uniques(metricName) FROM Metric SINCE 1 hour ago LIMIT 500";
        let result = self.nrql_query(nrql).await?;
        // Extract metricName strings from NerdGraph response structure.
        let names = result
            .pointer("/data/actor/account/nrql/results/0/uniques.metricName")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(names)
    }

    fn name(&self) -> &'static str {
        "new-relic"
    }
}

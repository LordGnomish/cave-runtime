//! Splunk HTTP Event Collector (HEC) adapter.
//!
//! Forwards logs to Splunk Enterprise or Splunk Cloud via the HEC endpoint.
//!
//! # Configuration
//!
//! ```toml
//! [logs]
//! backend           = "splunk"
//! splunk_hec_url    = "https://splunk.company.com:8088/services/collector/event"
//! splunk_hec_token  = "..."
//! splunk_index      = "cave_logs"
//! splunk_sourcetype = "cave:runtime"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::backend::{LogStreamBatch, LogsBackend, LogsBackendError, LogsResult};

#[derive(Debug, Clone, Deserialize)]
pub struct SplunkConfig {
    /// HEC endpoint URL, e.g. `https://splunk:8088/services/collector/event`.
    pub hec_url: String,
    /// HEC authentication token.
    pub hec_token: String,
    /// Target Splunk index.
    pub index: String,
    /// Sourcetype for log classification.
    pub sourcetype: String,
}

/// Single Splunk HEC event.
#[derive(Serialize)]
struct HecEvent<'a> {
    time: f64,
    host: &'a str,
    source: &'a str,
    sourcetype: &'a str,
    index: &'a str,
    fields: &'a std::collections::HashMap<String, String>,
    event: &'a str,
}

/// Splunk HEC adapter.
pub struct SplunkAdapter {
    config: SplunkConfig,
    client: reqwest::Client,
}

impl SplunkAdapter {
    pub fn new(config: SplunkConfig) -> Self {
        let client = reqwest::Client::builder()
            // Splunk self-signed certs are common in enterprise deployments.
            .danger_accept_invalid_certs(
                std::env::var("SPLUNK_INSECURE_TLS")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(false),
            )
            .build()
            .unwrap_or_default();
        Self { config, client }
    }

    async fn send_events(&self, events_ndjson: String) -> LogsResult<()> {
        let resp = self
            .client
            .post(&self.config.hec_url)
            .header("Authorization", format!("Splunk {}", self.config.hec_token))
            .header("Content-Type", "application/json")
            .body(events_ndjson)
            .send()
            .await
            .map_err(|e| LogsBackendError::Unreachable(format!("Splunk HEC request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LogsBackendError::PushFailed(format!(
                "Splunk HEC returned {status}: {body}"
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl LogsBackend for SplunkAdapter {
    async fn push(&self, streams: Vec<LogStreamBatch>) -> LogsResult<()> {
        // HEC batched format: newline-delimited JSON objects.
        // We chunk at 500 events to stay well under the 1MB default HEC limit.
        let mut batch_buf = String::with_capacity(65536);
        let mut batch_count = 0usize;

        for stream in &streams {
            let label_map: std::collections::HashMap<String, String> = stream
                .labels
                .0
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            let host = stream.labels.get("host").unwrap_or("cave-runtime");
            let source = stream
                .labels
                .get("job")
                .or_else(|| stream.labels.get("app"))
                .unwrap_or("cave");

            for entry in &stream.entries {
                let event = HecEvent {
                    time: entry.ts as f64 / 1_000_000_000.0,
                    host,
                    source,
                    sourcetype: &self.config.sourcetype,
                    index: &self.config.index,
                    fields: &label_map,
                    event: &entry.line,
                };

                if let Ok(json) = serde_json::to_string(&event) {
                    batch_buf.push_str(&json);
                    batch_buf.push('\n');
                    batch_count += 1;
                }

                if batch_count >= 500 {
                    self.send_events(std::mem::take(&mut batch_buf)).await?;
                    batch_count = 0;
                }
            }
        }

        if !batch_buf.is_empty() {
            self.send_events(batch_buf).await?;
        }

        Ok(())
    }

    async fn query(
        &self,
        _tenant_id: &str,
        _logql: &str,
        _limit: usize,
        _start_ns: i64,
        _end_ns: i64,
    ) -> LogsResult<serde_json::Value> {
        // Splunk search API requires a separate REST call chain:
        // POST /services/search/jobs → poll → GET /results.
        // Full implementation requires Splunk username/password or session key,
        // which is separate from HEC. Marked as future work.
        Err(LogsBackendError::QueryFailed(
            "SplunkAdapter: query not implemented — HEC is write-only. Use Splunk REST API for queries.".into(),
        ))
    }

    async fn label_names(&self, _tenant_id: &str) -> LogsResult<Vec<String>> {
        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "splunk"
    }
}

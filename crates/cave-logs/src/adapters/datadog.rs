//! Datadog Logs API adapter.
//!
//! Forwards logs to Datadog via the v2 Logs Ingestion API.
//!
//! # Configuration
//!
//! ```toml
//! [logs]
//! backend    = "datadog"
//! dd_api_key = "..."
//! dd_site    = "datadoghq.com"
//! dd_service = "cave-runtime"
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::backend::{LogStreamBatch, LogsBackend, LogsBackendError, LogsResult};

#[derive(Debug, Clone, Deserialize)]
pub struct DatadogLogsConfig {
    pub api_key: String,
    /// Datadog site, e.g. `datadoghq.com` or `datadoghq.eu`.
    pub site: String,
    /// Default service tag applied to all logs.
    pub service: String,
}

impl DatadogLogsConfig {
    pub fn logs_api_url(&self) -> String {
        format!("https://http-intake.logs.{}/api/v2/logs", self.site)
    }
}

/// Datadog v2 log entry.
#[derive(Serialize)]
struct DdLogEntry {
    ddsource: String,
    ddtags: String,
    hostname: String,
    message: String,
    service: String,
    timestamp: String,
}

/// Datadog Logs adapter.
pub struct DatadogLogsAdapter {
    config: DatadogLogsConfig,
    client: reqwest::Client,
}

impl DatadogLogsAdapter {
    pub fn new(config: DatadogLogsConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    async fn send_chunk(&self, entries: Vec<DdLogEntry>) -> LogsResult<()> {
        let resp = self
            .client
            .post(self.config.logs_api_url())
            .header("DD-API-KEY", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&entries)
            .send()
            .await
            .map_err(|e| LogsBackendError::Unreachable(format!("Datadog logs request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LogsBackendError::PushFailed(format!(
                "Datadog Logs returned {status}: {body}"
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl LogsBackend for DatadogLogsAdapter {
    async fn push(&self, streams: Vec<LogStreamBatch>) -> LogsResult<()> {
        let mut entries: Vec<DdLogEntry> = Vec::new();

        for stream in &streams {
            // Convert label set to `key:value,...` Datadog tags string.
            let ddtags = stream
                .labels
                .0
                .iter()
                .map(|(k, v)| format!("{}:{}", k, v))
                .collect::<Vec<_>>()
                .join(",");

            let service = stream
                .labels
                .get("app")
                .or_else(|| stream.labels.get("job"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| self.config.service.clone());

            let hostname = stream
                .labels
                .get("host")
                .map(|s| s.to_string())
                .unwrap_or_else(|| "cave-runtime".to_string());

            let source = stream
                .labels
                .get("__name__")
                .or_else(|| stream.labels.get("namespace"))
                .unwrap_or("cave");

            for entry in &stream.entries {
                let ts_nanos = entry.ts;
                let timestamp = chrono::DateTime::from_timestamp(
                    ts_nanos / 1_000_000_000,
                    (ts_nanos % 1_000_000_000) as u32,
                )
                .unwrap_or_else(Utc::now)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

                entries.push(DdLogEntry {
                    ddsource: source.to_string(),
                    ddtags: ddtags.clone(),
                    hostname: hostname.clone(),
                    message: entry.line.clone(),
                    service: service.clone(),
                    timestamp,
                });

                // Datadog accepts up to 1000 events per request.
                if entries.len() >= 1000 {
                    self.send_chunk(std::mem::take(&mut entries)).await?;
                }
            }
        }

        if !entries.is_empty() {
            self.send_chunk(entries).await?;
        }

        Ok(())
    }

    async fn query(
        &self,
        _tenant_id: &str,
        logql: &str,
        limit: usize,
        start_ns: i64,
        end_ns: i64,
    ) -> LogsResult<serde_json::Value> {
        // Datadog Logs Search API — translate simple filter to DD query syntax.
        let url = format!("https://api.{}/api/v2/logs/events/search", self.config.site);

        let from = chrono::DateTime::from_timestamp(start_ns / 1_000_000_000, 0)
            .unwrap_or_else(Utc::now)
            .to_rfc3339();
        let to = chrono::DateTime::from_timestamp(end_ns / 1_000_000_000, 0)
            .unwrap_or_else(Utc::now)
            .to_rfc3339();

        let payload = serde_json::json!({
            "filter": {
                "from": from,
                "to": to,
                // Pass the LogQL expression as a free-text Datadog query;
                // the caller should use DD query syntax when targeting this backend.
                "query": logql,
            },
            "page": { "limit": limit.min(1000) },
            "sort": "timestamp"
        });

        let resp = self
            .client
            .post(&url)
            .header("DD-API-KEY", &self.config.api_key)
            .header("DD-APPLICATION-KEY", std::env::var("DD_APP_KEY").unwrap_or_default())
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| LogsBackendError::QueryFailed(format!("Datadog logs query failed: {e}")))?;

        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| LogsBackendError::QueryFailed(format!("Datadog logs query parse error: {e}")))
    }

    async fn label_names(&self, _tenant_id: &str) -> LogsResult<Vec<String>> {
        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "datadog-logs"
    }
}

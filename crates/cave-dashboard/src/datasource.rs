//! Data-source abstraction for CAVE Dashboard.
//!
//! Supported backends:
//! - Prometheus  (via cave-metrics)  — PromQL queries
//! - Loki        (via cave-logs)     — LogQL queries
//! - Jaeger      (via cave-trace)    — trace / span queries

use reqwest::Client;

use crate::models::DataSourceType;

/// Generic data-source query response.
#[derive(Debug)]
pub struct QueryResult {
    pub ref_id: String,
    pub frames: Vec<DataFrame>,
}

/// A minimal data frame (series of float values).
#[derive(Debug)]
pub struct DataFrame {
    pub name: String,
    pub values: Vec<f64>,
    pub timestamps: Vec<i64>, // unix millis
}

// ─── Client ──────────────────────────────────────────────────────────────────

/// A client that can query a backing data source over HTTP.
pub struct DataSourceClient {
    pub ds_type: DataSourceType,
    pub base_url: String,
    client: Client,
}

impl DataSourceClient {
    pub fn new(ds_type: DataSourceType, base_url: impl Into<String>) -> Self {
        Self { ds_type, base_url: base_url.into(), client: Client::new() }
    }

    /// Issue a query to the data source and return raw frames.
    ///
    /// In a full implementation this calls the respective API:
    /// - Prometheus: `GET /api/v1/query_range?query=…&start=…&end=…&step=…`
    /// - Loki:       `GET /loki/api/v1/query_range?query=…`
    /// - Jaeger:     `GET /api/traces?service=…`
    pub async fn query(&self, expr: &str, _from: &str, _to: &str) -> anyhow::Result<QueryResult> {
        let url = self.build_url(expr);
        tracing::debug!(url = %url, ds_type = ?self.ds_type, "data-source query");

        // Stub: we build the URL but do not actually issue the HTTP request
        // (would require a running cave-metrics / cave-logs / cave-trace).
        let _ = self.client.get(&url); // keeps the reqwest::Client field live

        Ok(QueryResult {
            ref_id: "A".to_string(),
            frames: vec![DataFrame { name: expr.to_string(), values: vec![], timestamps: vec![] }],
        })
    }

    fn build_url(&self, expr: &str) -> String {
        match self.ds_type {
            DataSourceType::Prometheus => {
                format!("{}/api/v1/query?query={}", self.base_url, urlencoding(expr))
            }
            DataSourceType::Loki => {
                format!(
                    "{}/loki/api/v1/query_range?query={}",
                    self.base_url,
                    urlencoding(expr)
                )
            }
            DataSourceType::Jaeger => {
                format!("{}/api/traces?service={}", self.base_url, urlencoding(expr))
            }
            DataSourceType::Unknown => {
                format!("{}/query?q={}", self.base_url, urlencoding(expr))
            }
        }
    }
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}

//! AWS CloudWatch Metrics adapter.
//!
//! Forwards metrics to CloudWatch using the PutMetricData API via
//! HTTP + AWS Signature V4 signing (no heavy AWS SDK required).
//!
//! Credentials are resolved from the standard AWS credential chain:
//! `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` (+ optional `AWS_SESSION_TOKEN`).
//!
//! # Configuration
//!
//! ```toml
//! [metrics]
//! backend      = "cloudwatch"
//! cw_namespace = "CaveRuntime"
//! cw_region    = "us-east-1"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backend::{MetricsBackend, MetricsBackendError, MetricsResult};
use crate::model::TimeSeries;

#[derive(Debug, Clone, Deserialize)]
pub struct CloudWatchConfig {
    /// CloudWatch namespace, e.g. `"CaveRuntime/Metrics"`.
    pub namespace: String,
    /// AWS region, e.g. `"us-east-1"`.
    pub region: String,
}

impl CloudWatchConfig {
    pub fn endpoint(&self) -> String {
        format!("https://monitoring.{}.amazonaws.com/", self.region)
    }
}

/// AWS credentials loaded from environment.
struct AwsCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

impl AwsCredentials {
    fn from_env() -> Option<Self> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").ok()?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").ok()?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        Some(Self { access_key_id, secret_access_key, session_token })
    }
}

/// Sign a CloudWatch request using AWS Signature V4.
/// Returns (Authorization header, x-amz-date, x-amz-security-token if present).
fn sign_request(
    creds: &AwsCredentials,
    region: &str,
    method: &str,
    canonical_uri: &str,
    body: &str,
) -> HashMap<String, String> {
    use hmac::Mac;

    let now = chrono::Utc::now();
    let date_stamp = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

    let host = format!("monitoring.{}.amazonaws.com", region);
    let service = "monitoring";

    // Canonical headers
    let canonical_headers = if let Some(ref tok) = creds.session_token {
        format!(
            "host:{host}\nx-amz-date:{amz_date}\nx-amz-security-token:{tok}\n"
        )
    } else {
        format!("host:{host}\nx-amz-date:{amz_date}\n")
    };

    let signed_headers = if creds.session_token.is_some() {
        "host;x-amz-date;x-amz-security-token"
    } else {
        "host;x-amz-date"
    };

    // Body hash
    use sha2::Digest;
    let body_hash = format!("{:x}", sha2::Sha256::digest(body.as_bytes()));

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{body_hash}"
    );

    let credential_scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{:x}",
        sha2::Sha256::digest(canonical_request.as_bytes())
    );

    // Derive signing key
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
    )
    .unwrap();
    mac.update(date_stamp.as_bytes());
    let date_key = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&date_key).unwrap();
    mac.update(region.as_bytes());
    let region_key = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&region_key).unwrap();
    mac.update(service.as_bytes());
    let service_key = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&service_key).unwrap();
    mac.update(b"aws4_request");
    let signing_key = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&signing_key).unwrap();
    mac.update(string_to_sign.as_bytes());
    let signature = format!("{:x}", mac.finalize().into_bytes());

    let auth = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        creds.access_key_id, credential_scope, signed_headers, signature
    );

    let mut headers = HashMap::new();
    headers.insert("Authorization".into(), auth);
    headers.insert("x-amz-date".into(), amz_date);
    if let Some(tok) = &creds.session_token {
        headers.insert("x-amz-security-token".into(), tok.clone());
    }
    headers
}

/// AWS CloudWatch Metrics adapter.
pub struct CloudWatchAdapter {
    config: CloudWatchConfig,
    client: reqwest::Client,
}

impl CloudWatchAdapter {
    pub fn new(config: CloudWatchConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    /// Build URL-encoded PutMetricData body.
    fn build_put_metric_data_body(&self, chunk: &[TimeSeries]) -> String {
        let mut params: Vec<(String, String)> = vec![
            ("Action".into(), "PutMetricData".into()),
            ("Version".into(), "2010-08-01".into()),
            ("Namespace".into(), self.config.namespace.clone()),
        ];

        let mut idx = 1usize;
        for ts in chunk {
            let metric_name = ts.labels.get("__name__").unwrap_or("Metric").to_string();

            // Dimensions from labels (excluding __name__), max 30.
            let dims: Vec<(&str, &str)> = ts
                .labels
                .0
                .iter()
                .filter(|(k, _)| k.as_str() != "__name__")
                .take(30)
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            for (sample_idx, sample) in ts.samples.iter().enumerate() {
                let m_prefix = format!("MetricData.member.{}", idx);
                params.push((format!("{}.MetricName", m_prefix), metric_name.clone()));
                params.push((format!("{}.Value", m_prefix), sample.value.to_string()));
                params.push((
                    format!("{}.Timestamp", m_prefix),
                    chrono::DateTime::from_timestamp_millis(sample.timestamp_ms)
                        .unwrap_or_else(chrono::Utc::now)
                        .format("%Y-%m-%dT%H:%M:%SZ")
                        .to_string(),
                ));
                params.push((format!("{}.Unit", m_prefix), "None".into()));

                for (di, (dk, dv)) in dims.iter().enumerate() {
                    let d_prefix = format!("{}.Dimensions.member.{}", m_prefix, di + 1);
                    params.push((format!("{}.Name", d_prefix), dk.to_string()));
                    params.push((format!("{}.Value", d_prefix), dv.to_string()));
                }

                idx += 1;
                // CloudWatch accepts max 20 MetricData items per call.
                if idx > 20 { break; }
            }
            if idx > 20 { break; }
        }

        params
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    urlencoding_simple(k),
                    urlencoding_simple(v)
                )
            })
            .collect::<Vec<_>>()
            .join("&")
    }
}

fn urlencoding_simple(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            _ => format!("%{:02X}", c as u32).chars().collect::<Vec<_>>(),
        })
        .collect()
}

#[async_trait]
impl MetricsBackend for CloudWatchAdapter {
    async fn write(&self, batch: Vec<TimeSeries>) -> MetricsResult<()> {
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(MetricsBackendError::ConfigError(
                "AWS credentials not found. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.".into(),
            ));
        };

        // CloudWatch PutMetricData: max 20 metric data per call.
        for chunk in batch.chunks(20) {
            let body = self.build_put_metric_data_body(chunk);
            let sig_headers =
                sign_request(&creds, &self.config.region, "POST", "/", &body);

            let mut req = self
                .client
                .post(self.config.endpoint())
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Host", format!("monitoring.{}.amazonaws.com", self.config.region))
                .body(body);

            for (k, v) in &sig_headers {
                req = req.header(k.as_str(), v.as_str());
            }

            let resp = req
                .send()
                .await
                .map_err(|e| MetricsBackendError::Unreachable(format!("CloudWatch request failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(MetricsBackendError::WriteFailed(format!(
                    "CloudWatch returned {status}: {body}"
                )));
            }
        }

        Ok(())
    }

    async fn query_instant(&self, expr: &str, timestamp_ms: i64) -> MetricsResult<serde_json::Value> {
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(MetricsBackendError::ConfigError("AWS credentials not found".into()));
        };

        let start = chrono::DateTime::from_timestamp_millis(timestamp_ms - 60_000)
            .unwrap_or_else(chrono::Utc::now)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let end = chrono::DateTime::from_timestamp_millis(timestamp_ms)
            .unwrap_or_else(chrono::Utc::now)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let body = format!(
            "Action=GetMetricData&Version=2010-08-01\
             &MetricDataQueries.member.1.Id=q1\
             &MetricDataQueries.member.1.Expression={}\
             &StartTime={}&EndTime={}\
             &ScanBy=TimestampDescending",
            urlencoding_simple(expr),
            urlencoding_simple(&start),
            urlencoding_simple(&end),
        );

        let sig_headers = sign_request(&creds, &self.config.region, "POST", "/", &body);

        let mut req = self
            .client
            .post(self.config.endpoint())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body);

        for (k, v) in &sig_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| MetricsBackendError::QueryFailed(format!("CloudWatch query failed: {e}")))?;

        let text = resp.text().await.unwrap_or_default();
        Ok(serde_json::json!({ "raw": text }))
    }

    async fn query_range(
        &self,
        expr: &str,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> MetricsResult<serde_json::Value> {
        // GetMetricData supports time ranges natively; reuse query_instant logic.
        self.query_instant(expr, end_ms).await
    }

    async fn label_names(&self) -> MetricsResult<Vec<String>> {
        // CloudWatch doesn't have a generic label-name API; return namespace.
        Ok(vec![self.config.namespace.clone()])
    }

    fn name(&self) -> &'static str {
        "cloudwatch"
    }
}

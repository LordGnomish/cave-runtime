//! AWS CloudWatch Logs adapter.
//!
//! Forwards logs to CloudWatch Logs via the PutLogEvents API,
//! signed with AWS Signature V4 (no SDK dependency — uses reqwest + sha2/hmac).
//!
//! # Configuration
//!
//! ```toml
//! [logs]
//! backend        = "cloudwatch"
//! cw_log_group   = "/cave/runtime"
//! cw_log_stream  = "default"
//! cw_region      = "us-east-1"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backend::{LogStreamBatch, LogsBackend, LogsBackendError, LogsResult};

#[derive(Debug, Clone, Deserialize)]
pub struct CloudWatchLogsConfig {
    pub log_group: String,
    pub log_stream: String,
    pub region: String,
}

impl CloudWatchLogsConfig {
    pub fn endpoint(&self) -> String {
        format!("https://logs.{}.amazonaws.com/", self.region)
    }
}

/// CloudWatch Logs PutLogEvents input log event.
#[derive(Serialize)]
struct InputLogEvent {
    message: String,
    timestamp: i64, // milliseconds since epoch
}

/// PutLogEvents request body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PutLogEventsRequest<'a> {
    log_group_name: &'a str,
    log_stream_name: &'a str,
    log_events: Vec<InputLogEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence_token: Option<String>,
}

struct AwsCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

impl AwsCredentials {
    fn from_env() -> Option<Self> {
        Some(Self {
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok()?,
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok()?,
            session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
        })
    }
}

fn sign_cloudwatch_logs(
    creds: &AwsCredentials,
    region: &str,
    body: &str,
    target: &str,
) -> HashMap<String, String> {
    use hmac::Mac;
    use sha2::Digest;

    let now = chrono::Utc::now();
    let date_stamp = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

    let host = format!("logs.{}.amazonaws.com", region);
    let service = "logs";

    let body_hash = format!("{:x}", sha2::Sha256::digest(body.as_bytes()));

    let canonical_headers_base =
        format!("content-type:application/x-amz-json-1.1\nhost:{host}\nx-amz-date:{amz_date}\nx-amz-target:{target}\n");
    let signed_headers_base = "content-type;host;x-amz-date;x-amz-target";

    let (canonical_headers, signed_headers) = if let Some(ref tok) = creds.session_token {
        (
            format!("{}x-amz-security-token:{tok}\n", canonical_headers_base),
            format!("{};x-amz-security-token", signed_headers_base),
        )
    } else {
        (canonical_headers_base, signed_headers_base.to_string())
    };

    let canonical_request = format!(
        "POST\n/\n\n{canonical_headers}\n{signed_headers}\n{body_hash}"
    );

    let credential_scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{:x}",
        sha2::Sha256::digest(canonical_request.as_bytes())
    );

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
    )
    .unwrap();
    mac.update(date_stamp.as_bytes());
    let dk = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&dk).unwrap();
    mac.update(region.as_bytes());
    let rk = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&rk).unwrap();
    mac.update(service.as_bytes());
    let sk = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&sk).unwrap();
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

/// CloudWatch Logs sequence token cache (thread-local for simplicity).
static SEQUENCE_TOKEN: std::sync::OnceLock<tokio::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

fn sequence_token_lock() -> &'static tokio::sync::Mutex<Option<String>> {
    SEQUENCE_TOKEN.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// AWS CloudWatch Logs adapter.
pub struct CloudWatchLogsAdapter {
    config: CloudWatchLogsConfig,
    client: reqwest::Client,
}

impl CloudWatchLogsAdapter {
    pub fn new(config: CloudWatchLogsConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    async fn put_log_events(
        &self,
        creds: &AwsCredentials,
        events: Vec<InputLogEvent>,
    ) -> LogsResult<Option<String>> {
        let mut token_guard = sequence_token_lock().lock().await;

        let req_body = PutLogEventsRequest {
            log_group_name: &self.config.log_group,
            log_stream_name: &self.config.log_stream,
            log_events: events,
            sequence_token: token_guard.clone(),
        };

        let body = serde_json::to_string(&req_body)
            .map_err(|e| LogsBackendError::PushFailed(format!("Serialization error: {e}")))?;

        let target = "Logs_20140328.PutLogEvents";
        let sig_headers = sign_cloudwatch_logs(creds, &self.config.region, &body, target);

        let mut req = self
            .client
            .post(self.config.endpoint())
            .header("Content-Type", "application/x-amz-json-1.1")
            .header("x-amz-target", target)
            .body(body);

        for (k, v) in &sig_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| LogsBackendError::Unreachable(format!("CloudWatch Logs request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(LogsBackendError::PushFailed(format!(
                "CloudWatch Logs returned {status}: {body_text}"
            )));
        }

        // Extract next sequence token from response.
        let response_json: serde_json::Value = resp.json().await.unwrap_or_default();
        let next_token = response_json
            .get("nextSequenceToken")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        *token_guard = next_token.clone();
        Ok(next_token)
    }
}

#[async_trait]
impl LogsBackend for CloudWatchLogsAdapter {
    async fn push(&self, streams: Vec<LogStreamBatch>) -> LogsResult<()> {
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(LogsBackendError::Unreachable(
                "AWS credentials not found. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.".into(),
            ));
        };

        let mut events: Vec<InputLogEvent> = Vec::new();

        for stream in &streams {
            for entry in &stream.entries {
                events.push(InputLogEvent {
                    message: entry.line.clone(),
                    timestamp: entry.ts / 1_000_000, // ns → ms
                });

                // CloudWatch Logs accepts max 10,000 events or 1MB per call.
                if events.len() >= 10_000 {
                    self.put_log_events(&creds, std::mem::take(&mut events)).await?;
                }
            }
        }

        if !events.is_empty() {
            self.put_log_events(&creds, events).await?;
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
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(LogsBackendError::Unreachable("AWS credentials not found".into()));
        };

        // CloudWatch Logs Insights StartQuery.
        let target = "Logs_20140328.StartQuery";

        let start_time_sec = start_ns / 1_000_000_000;
        let end_time_sec = end_ns / 1_000_000_000;

        let start_body = serde_json::json!({
            "logGroupName": self.config.log_group,
            // Pass LogQL through as Insights query syntax when using this backend.
            "queryString": logql,
            "startTime": start_time_sec,
            "endTime": end_time_sec,
            "limit": limit.min(10_000),
        })
        .to_string();

        let sig_headers = sign_cloudwatch_logs(&creds, &self.config.region, &start_body, target);

        let mut req = self
            .client
            .post(self.config.endpoint())
            .header("Content-Type", "application/x-amz-json-1.1")
            .header("x-amz-target", target)
            .body(start_body.clone());

        for (k, v) in &sig_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let start_resp = req
            .send()
            .await
            .map_err(|e| LogsBackendError::QueryFailed(format!("CW Logs StartQuery failed: {e}")))?;

        let start_json: serde_json::Value = start_resp.json().await.unwrap_or_default();
        let query_id = start_json
            .get("queryId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LogsBackendError::QueryFailed("CW Logs: no queryId in response".into()))?
            .to_string();

        // Poll GetQueryResults up to 10 times (Insights queries are async).
        let results_target = "Logs_20140328.GetQueryResults";
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let results_body = serde_json::json!({ "queryId": query_id }).to_string();
            let sig_h =
                sign_cloudwatch_logs(&creds, &self.config.region, &results_body, results_target);

            let mut req = self
                .client
                .post(self.config.endpoint())
                .header("Content-Type", "application/x-amz-json-1.1")
                .header("x-amz-target", results_target)
                .body(results_body);

            for (k, v) in &sig_h {
                req = req.header(k.as_str(), v.as_str());
            }

            let results_resp = req
                .send()
                .await
                .map_err(|e| LogsBackendError::QueryFailed(format!("CW Logs GetQueryResults failed: {e}")))?;

            let results_json: serde_json::Value = results_resp.json().await.unwrap_or_default();

            let status = results_json
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");

            if status == "Complete" || status == "Failed" || status == "Cancelled" {
                return Ok(results_json);
            }
        }

        Err(LogsBackendError::QueryFailed(
            "CloudWatch Logs Insights query timed out".into(),
        ))
    }

    async fn label_names(&self, _tenant_id: &str) -> LogsResult<Vec<String>> {
        Ok(vec![
            "log_group".to_string(),
            "log_stream".to_string(),
            "region".to_string(),
        ])
    }

    fn name(&self) -> &'static str {
        "cloudwatch-logs"
    }
}

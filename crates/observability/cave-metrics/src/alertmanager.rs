// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Alertmanager client.

#![allow(dead_code)]

use std::time::Duration;
use serde::Serialize;
use crate::error::{MetricsError, MetricsResult};
use crate::model::LabelMatcher;
use crate::rules::Alert;

#[derive(Debug, Clone)]
pub struct AlertmanagerConfig {
    pub url: String,
    pub timeout_ms: u64,
}

pub struct AlertmanagerClient {
    config: AlertmanagerConfig,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct AlertPayload {
    labels: std::collections::HashMap<String, String>,
    annotations: std::collections::HashMap<String, String>,
    #[serde(rename = "startsAt")]
    starts_at: String,
    #[serde(rename = "endsAt", skip_serializing_if = "Option::is_none")]
    ends_at: Option<String>,
}

#[derive(Serialize)]
struct SilenceMatcher {
    name: String,
    value: String,
    #[serde(rename = "isRegex")]
    is_regex: bool,
    #[serde(rename = "isEqual")]
    is_equal: bool,
}

#[derive(Serialize)]
struct SilencePayload {
    matchers: Vec<SilenceMatcher>,
    #[serde(rename = "startsAt")]
    starts_at: String,
    #[serde(rename = "endsAt")]
    ends_at: String,
    comment: String,
    #[serde(rename = "createdBy")]
    created_by: String,
}

fn ms_to_rfc3339(ms: i64) -> String {
    use chrono::{DateTime, Utc, TimeZone};
    let dt: DateTime<Utc> = Utc.timestamp_millis_opt(ms).unwrap();
    dt.to_rfc3339()
}

impl AlertmanagerClient {
    pub fn new(config: AlertmanagerConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .unwrap_or_default();
        Self { config, client }
    }

    pub async fn send_alerts(&self, alerts: &[Alert]) -> MetricsResult<()> {
        let payloads: Vec<AlertPayload> = alerts.iter().map(|a| AlertPayload {
            labels: a.labels.0.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            annotations: a.annotations.0.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            starts_at: ms_to_rfc3339(a.fired_at),
            ends_at: a.resolved_at.map(ms_to_rfc3339),
        }).collect();

        let url = format!("{}/api/v2/alerts", self.config.url);
        self.client
            .post(&url)
            .json(&payloads)
            .send()
            .await
            .map_err(MetricsError::Http)?;
        Ok(())
    }

    pub async fn create_silence(
        &self,
        matchers: &[LabelMatcher],
        starts_at: i64,
        ends_at: i64,
        comment: &str,
    ) -> MetricsResult<String> {
        let sm: Vec<SilenceMatcher> = matchers.iter().map(|m| {
            let (name, value, is_regex, is_equal) = match m {
                LabelMatcher::Equal { name, value } => (name.clone(), value.clone(), false, true),
                LabelMatcher::NotEqual { name, value } => (name.clone(), value.clone(), false, false),
                LabelMatcher::RegexMatch { name, pattern } => (name.clone(), pattern.clone(), true, true),
                LabelMatcher::RegexNotMatch { name, pattern } => (name.clone(), pattern.clone(), true, false),
            };
            SilenceMatcher { name, value, is_regex, is_equal }
        }).collect();

        let payload = SilencePayload {
            matchers: sm,
            starts_at: ms_to_rfc3339(starts_at),
            ends_at: ms_to_rfc3339(ends_at),
            comment: comment.to_string(),
            created_by: "cave-metrics".to_string(),
        };

        let url = format!("{}/api/v2/silences", self.config.url);
        let resp = self.client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(MetricsError::Http)?;

        let body: serde_json::Value = resp.json().await.map_err(MetricsError::Http)?;
        let silence_id = body["silenceID"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(silence_id)
    }
}

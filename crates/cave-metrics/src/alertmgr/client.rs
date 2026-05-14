// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AlertManager HTTP client for pushing alerts and creating silences.

use crate::error::{MetricsError, Result};
use crate::rules::FiringAlert;
use super::model::{Alert, Silence};

pub struct AlertmanagerClient {
    base_url: String,
    client: reqwest::Client,
}

impl AlertmanagerClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Push a batch of alerts to AlertManager.
    pub async fn send_alerts(&self, alerts: &[Alert]) -> Result<()> {
        let url = format!("{}/api/v2/alerts", self.base_url);
        self.client.post(&url)
            .json(alerts)
            .send()
            .await
            .map_err(|e| MetricsError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| MetricsError::Http(e.to_string()))?;
        Ok(())
    }

    /// Create a silence on AlertManager.
    pub async fn create_silence(&self, silence: &Silence) -> Result<String> {
        let url = format!("{}/api/v2/silences", self.base_url);
        let resp: serde_json::Value = self.client.post(&url)
            .json(silence)
            .send()
            .await
            .map_err(|e| MetricsError::Http(e.to_string()))?
            .json()
            .await
            .map_err(|e| MetricsError::Http(e.to_string()))?;
        Ok(resp["silenceID"].as_str().unwrap_or("").to_string())
    }

    /// Delete (expire) a silence.
    pub async fn delete_silence(&self, id: &str) -> Result<()> {
        let url = format!("{}/api/v2/silence/{}", self.base_url, id);
        self.client.delete(&url)
            .send()
            .await
            .map_err(|e| MetricsError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| MetricsError::Http(e.to_string()))?;
        Ok(())
    }

    /// List all silences.
    pub async fn list_silences(&self) -> Result<Vec<Silence>> {
        let url = format!("{}/api/v2/silences", self.base_url);
        let resp: Vec<Silence> = self.client.get(&url)
            .send()
            .await
            .map_err(|e| MetricsError::Http(e.to_string()))?
            .json()
            .await
            .map_err(|e| MetricsError::Http(e.to_string()))?;
        Ok(resp)
    }
}

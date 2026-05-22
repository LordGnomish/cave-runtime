// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scrape target configuration and state.

use crate::model::Labels;
use serde::{Deserialize, Serialize};

/// Configuration for a single scrape job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeConfig {
    pub job_name: String,
    pub scrape_interval_ms: i64,
    pub scrape_timeout_ms: i64,
    pub metrics_path: String,
    pub scheme: String, // http / https
    pub honor_labels: bool,
    pub honor_timestamps: bool,
    pub static_configs: Vec<StaticConfig>,
    pub file_sd_configs: Vec<FileSdConfig>,
    pub kubernetes_sd_configs: Vec<KubernetesSdConfig>,
}

impl Default for ScrapeConfig {
    fn default() -> Self {
        Self {
            job_name: "unknown".to_string(),
            scrape_interval_ms: 15_000,
            scrape_timeout_ms: 10_000,
            metrics_path: "/metrics".to_string(),
            scheme: "http".to_string(),
            honor_labels: false,
            honor_timestamps: true,
            static_configs: Vec::new(),
            file_sd_configs: Vec::new(),
            kubernetes_sd_configs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticConfig {
    pub targets: Vec<String>, // host:port
    pub labels: Labels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSdConfig {
    pub files: Vec<String>,
    pub refresh_interval_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubernetesSdConfig {
    pub role: K8sRole,
    pub namespaces: Vec<String>,
    pub selectors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum K8sRole {
    Node,
    Pod,
    Service,
    Endpoints,
    EndpointSlice,
    Ingress,
}

/// A single resolved scrape target.
#[derive(Debug, Clone)]
pub struct ScrapeTarget {
    pub url: String,
    pub labels: Labels, // discovered + job labels
    pub config: ScrapeConfig,
    pub last_scrape_ms: i64,
    pub last_error: Option<String>,
    pub last_duration_ms: i64,
}

impl ScrapeTarget {
    pub fn new(url: impl Into<String>, labels: Labels, config: ScrapeConfig) -> Self {
        Self {
            url: url.into(),
            labels,
            config,
            last_scrape_ms: 0,
            last_error: None,
            last_duration_ms: 0,
        }
    }

    pub fn health(&self) -> &str {
        match &self.last_error {
            None if self.last_scrape_ms > 0 => "up",
            Some(_) => "down",
            None => "unknown",
        }
    }
}

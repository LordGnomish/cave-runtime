// SPDX-License-Identifier: AGPL-3.0-or-later
//! Data models for cave-upstream.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamType {
    ExternalApi,
    ManagedService,
    OpenSourceLib,
    InternalService,
    CloudProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamStatus {
    Operational,
    Degraded,
    Incident,
    Deprecated,
    Eol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SupportTier {
    Community,
    Commercial,
    Enterprise,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamAlertType {
    StatusChange,
    DeprecationWarning,
    EolWarning,
    CostThreshold,
    HighLatency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamService {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub upstream_type: UpstreamType,
    pub vendor: Option<String>,
    pub version: Option<String>,
    pub status: UpstreamStatus,
    pub health_check_url: Option<String>,
    pub docs_url: Option<String>,
    pub license: Option<String>,
    pub support_tier: SupportTier,
    pub cost_per_month_usd: Option<f64>,
    pub deprecation_date: Option<DateTime<Utc>>,
    pub eol_date: Option<DateTime<Utc>>,
    pub alternatives: Vec<String>,
    pub tags: Vec<String>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub upstream_id: Uuid,
    pub checked_at: DateTime<Utc>,
    pub latency_ms: u64,
    pub status: UpstreamStatus,
    pub error: Option<String>,
    pub response_code: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamAlert {
    pub upstream_id: Uuid,
    pub alert_type: UpstreamAlertType,
    pub message: String,
    pub severity: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamStats {
    pub total: u64,
    pub operational: u64,
    pub degraded: u64,
    pub incidents: u64,
    pub deprecated: u64,
    pub eol: u64,
    pub by_type: HashMap<String, u64>,
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeploymentRecord {
    pub id: Uuid,
    pub pipeline: String,
    pub environment: String,
    pub status: DeployStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub commit_sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeployStatus {
    Success,
    Failure,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DoraMetrics {
    pub team: String,
    pub deployment_frequency: f64,
    pub lead_time_hours: f64,
    pub change_failure_rate: f64,
    pub mttr_hours: f64,
}

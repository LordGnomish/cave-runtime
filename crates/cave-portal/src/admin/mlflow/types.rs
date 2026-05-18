// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared types for the `/admin/mlflow` page set. Mirrors mlflow/mlflow
//! v2.x REST API shapes (experiments, runs, registered models, model
//! versions, deployments).

use crate::admin::types::TenantId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlflowExperiment {
    pub tenant: TenantId,
    pub experiment_id: String,
    pub name: String,
    pub artifact_location: String,
    /// "active" | "deleted"
    pub lifecycle_stage: String,
    pub creation_time_ms: i64,
    pub last_update_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MlflowRun {
    pub tenant: TenantId,
    pub run_id: String,
    pub experiment_id: String,
    pub user: String,
    /// "RUNNING" | "FINISHED" | "FAILED" | "KILLED" | "SCHEDULED"
    pub status: String,
    pub start_time_ms: i64,
    pub end_time_ms: Option<i64>,
    pub artifact_uri: String,
    pub primary_metric: Option<(String, f64)>,
    pub run_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredModel {
    pub tenant: TenantId,
    pub name: String,
    pub creation_time_ms: i64,
    pub last_updated_ms: i64,
    pub description: String,
    pub latest_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVersion {
    pub tenant: TenantId,
    pub registered_model_name: String,
    pub version: u32,
    /// "None" | "Staging" | "Production" | "Archived"
    pub current_stage: String,
    pub source_run_id: String,
    pub creation_time_ms: i64,
    /// "READY" | "PENDING" | "FAILED"
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDeployment {
    pub tenant: TenantId,
    pub deployment_name: String,
    pub registered_model_name: String,
    pub model_version: u32,
    /// "READY" | "PENDING" | "FAILED" | "UPDATING"
    pub status: String,
    pub endpoint_url: String,
    pub deployed_at_ms: i64,
    pub last_request_unix: Option<i64>,
    pub request_count_24h: u64,
    pub p95_latency_ms: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MlflowViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("experiment {0} not found")]
    ExperimentNotFound(String),
    #[error("run {0} not found")]
    RunNotFound(String),
    #[error("model {0} not found")]
    ModelNotFound(String),
}

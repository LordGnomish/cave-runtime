//! Domain models for cave-deploy.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Sources ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSource {
    pub repo_url: String,
    pub branch: String,
    pub path: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmSource {
    pub repo_url: String,
    pub chart: String,
    pub version: String,
    pub values: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KustomizeSource {
    pub repo_url: String,
    pub path: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApplicationSource {
    Git(GitSource),
    Helm(HelmSource),
    Kustomize(KustomizeSource),
}

// ─── Enums ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncPolicy {
    Manual,
    Automated,
    AutomatedWithPrune,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Progressing,
    Degraded,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Progressing,
    Degraded,
    Suspended,
    Missing,
    Unknown,
}

// ─── Application ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub source: ApplicationSource,
    pub target_cluster: String,
    pub sync_policy: SyncPolicy,
    pub sync_status: SyncStatus,
    pub health_status: HealthStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub revision: Option<String>,
    pub message: Option<String>,
}

// ─── Rollout ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStrategy {
    Canary,
    BlueGreen,
    Rolling,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutStep {
    pub step_index: usize,
    pub weight: u8,
    pub pause_duration_secs: Option<u64>,
    pub analysis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStatus {
    Pending,
    Progressing,
    Paused,
    Promoting,
    Aborting,
    Completed,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollout {
    pub id: Uuid,
    pub application_id: Uuid,
    pub strategy: RolloutStrategy,
    pub status: RolloutStatus,
    pub current_step: usize,
    pub steps: Vec<RolloutStep>,
    pub stable_revision: String,
    pub canary_revision: String,
    pub traffic_weight: u8,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error: Option<String>,
}

// ─── Deployment / History ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: Uuid,
    pub application_id: Uuid,
    pub revision: String,
    pub sync_status: SyncStatus,
    pub health_status: HealthStatus,
    pub deployed_at: DateTime<Utc>,
    pub deployed_by: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentHistory {
    pub application_id: Uuid,
    pub entries: Vec<Deployment>,
}

// ─── Resource / Diff ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStatus {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub health: HealthStatus,
    pub sync_status: SyncStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDiff {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub application_id: Uuid,
    pub has_diff: bool,
    pub resources: Vec<ResourceDiff>,
    pub generated_at: DateTime<Utc>,
}

// ─── In-memory store ─────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DeployStore {
    pub applications: HashMap<Uuid, Application>,
    pub rollouts: HashMap<Uuid, Rollout>,
    pub history: HashMap<Uuid, Vec<Deployment>>,
}

// ─── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateApplicationRequest {
    pub name: String,
    pub namespace: String,
    pub source: ApplicationSource,
    pub target_cluster: String,
    pub sync_policy: Option<SyncPolicy>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApplicationRequest {
    pub name: Option<String>,
    pub source: Option<ApplicationSource>,
    pub sync_policy: Option<SyncPolicy>,
    pub target_cluster: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    pub revision: Option<String>,
    pub force: Option<bool>,
    #[allow(dead_code)]
    pub prune: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RollbackRequest {
    pub deployment_id: Option<Uuid>,
    pub revision: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RolloutStepRequest {
    pub weight: u8,
    pub pause_duration_secs: Option<u64>,
    pub analysis: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRolloutRequest {
    pub application_id: Uuid,
    pub strategy: RolloutStrategy,
    pub canary_revision: String,
    pub steps: Vec<RolloutStepRequest>,
}

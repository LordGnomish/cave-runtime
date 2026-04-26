use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaledObject {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub target_ref: ScaleTargetRef,
    pub triggers: Vec<ScaleTrigger>,
    pub min_replica_count: Option<u32>,
    pub max_replica_count: u32,
    pub polling_interval_secs: u32,
    pub cooldown_period_secs: u32,
    pub status: ScaledObjectStatus,
    pub current_replicas: u32,
    pub desired_replicas: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaledJob {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub job_template: JobTemplate,
    pub triggers: Vec<ScaleTrigger>,
    pub max_replica_count: u32,
    pub polling_interval_secs: u32,
    pub status: ScaledJobStatus,
    pub active_jobs: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleTargetRef {
    pub api_version: Option<String>,
    pub kind: String,
    pub name: String,
    pub env_source_container_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTemplate {
    pub spec: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleTrigger {
    pub trigger_type: String,
    pub name: Option<String>,
    pub metadata: HashMap<String, String>,
    pub auth_ref: Option<TriggerAuthRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerAuthRef {
    pub name: String,
    pub kind: TriggerAuthKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerAuthKind {
    TriggerAuthentication,
    ClusterTriggerAuthentication,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerAuthentication {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub spec: TriggerAuthSpec,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerAuthSpec {
    pub secret_target_ref: Vec<SecretTargetRef>,
    pub env: Vec<AuthEnvRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretTargetRef {
    pub parameter: String,
    pub name: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthEnvRef {
    pub parameter: String,
    pub name: String,
    pub container_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScaledObjectStatus {
    Active,
    Inactive,
    Paused,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScaledJobStatus {
    Active,
    Idle,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricValue {
    pub trigger_name: String,
    pub trigger_type: String,
    pub metric_name: String,
    pub current_value: f64,
    pub target_value: f64,
    pub is_active: bool,
    pub measured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScaledObjectRequest {
    pub name: String,
    pub namespace: String,
    pub target_ref: ScaleTargetRef,
    pub triggers: Vec<ScaleTrigger>,
    pub min_replica_count: Option<u32>,
    pub max_replica_count: Option<u32>,
    pub polling_interval_secs: Option<u32>,
    pub cooldown_period_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScaledJobRequest {
    pub name: String,
    pub namespace: String,
    pub job_template: JobTemplate,
    pub triggers: Vec<ScaleTrigger>,
    pub max_replica_count: Option<u32>,
    pub polling_interval_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTriggerAuthRequest {
    pub name: String,
    pub namespace: String,
    pub spec: TriggerAuthSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleRequest {
    pub desired_replicas: u32,
}

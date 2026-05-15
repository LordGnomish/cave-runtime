// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Progressive delivery models — Flagger + Argo Rollouts parity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Rollout top-level ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollout {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub workload_ref: WorkloadRef,
    pub strategy: RolloutStrategy,
    pub status: RolloutStatus,
    pub traffic: Option<TrafficConfig>,
    pub analysis: Option<AnalysisRef>,
    pub notifications: Vec<NotificationTarget>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadRef {
    pub kind: String, // "Deployment" | "StatefulSet"
    pub name: String,
    pub namespace: String,
    pub image: String,
    pub replicas: i32,
}

// ── Strategies ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RolloutStrategy {
    Canary(CanaryStrategy),
    BlueGreen(BlueGreenStrategy),
    ABTest(ABTestStrategy),
}

/// Weight-based step-by-step traffic shifting with metric analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryStrategy {
    /// Ordered steps: weight increments and analysis phases.
    pub steps: Vec<CanaryStep>,
    /// Target stable service name.
    pub stable_service: String,
    /// Canary service name.
    pub canary_service: String,
    /// Max weight the canary will receive (1-100).
    pub max_weight: u8,
    /// How long to wait between steps if not using analysis.
    pub step_weight_increment: u8,
    /// Threshold below which the rollout is automatically aborted.
    pub threshold: Option<MetricThreshold>,
    /// How many failed metric checks before aborting.
    pub max_analysis_failures: Option<u32>,
    /// Traffic mirror percentage (0 = disabled).
    pub mirror_percentage: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CanaryStep {
    /// Shift `weight` % of traffic to canary.
    SetWeight { weight: u8 },
    /// Pause until manually promoted or for `duration` seconds.
    Pause { duration_seconds: Option<u64> },
    /// Run analysis before proceeding.
    Analysis { template_name: String },
    /// Mirror `percentage` % of traffic to canary (read-only).
    SetMirrorWeight { percentage: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricThreshold {
    pub metric: String,
    pub threshold_range: ThresholdRange,
    pub interval: String, // e.g. "30s"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Atomic blue/green: stand up a preview, verify, then cut over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueGreenStrategy {
    pub active_service: String,
    pub preview_service: String,
    /// Seconds to keep the old (inactive) ReplicaSet before scale-down.
    pub scale_down_delay_seconds: u64,
    /// Auto-promote after this many seconds of passing analysis (0 = manual).
    pub auto_promote_seconds: u64,
    pub pre_promotion_analysis: Option<String>,
    pub post_promotion_analysis: Option<String>,
    pub anti_affinity: Option<AntiAffinity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntiAffinity {
    pub preferred_during_scheduling: bool,
    pub required_during_scheduling: bool,
}

/// Header-based routing for A/B experiments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABTestStrategy {
    pub stable_service: String,
    pub canary_service: String,
    pub header_rules: Vec<HeaderRule>,
    pub max_weight: u8,
    pub step_weight: u8,
    pub analysis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderRule {
    pub name: String,
    pub value: String,
    /// "exact" | "prefix" | "regex"
    pub match_type: String,
}

// ── Analysis ──────────────────────────────────────────────────────────────────

/// Reusable analysis definition (like Argo AnalysisTemplate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisTemplate {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub metrics: Vec<MetricSpec>,
    pub dry_run_metrics: Vec<String>,
    pub args: Vec<AnalysisArg>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisArg {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSpec {
    pub name: String,
    pub provider: MetricProvider,
    pub success_condition: Option<String>,
    pub failure_condition: Option<String>,
    pub failure_limit: Option<u32>,
    pub count: Option<u32>,
    pub interval: Option<String>,
    pub initial_delay: Option<String>,
    pub consecutive_error_limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MetricProvider {
    Prometheus {
        address: String,
        query: String,
        timeout: Option<String>,
    },
    Webhook {
        url: String,
        method: Option<String>,
        headers: Option<HashMap<String, String>>,
        body: Option<serde_json::Value>,
        timeout_seconds: Option<u64>,
    },
    Datadog {
        query: String,
        api_version: Option<String>,
        interval: Option<String>,
        use_extended_metrics: Option<bool>,
    },
    NewRelic {
        profile: Option<String>,
        query: String,
    },
    CloudWatch {
        metric_data_queries: Vec<serde_json::Value>,
    },
    Job {
        spec: serde_json::Value,
    },
}

/// A concrete execution of an AnalysisTemplate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRun {
    pub id: Uuid,
    pub rollout_id: Uuid,
    pub template_name: String,
    pub phase: AnalysisPhase,
    pub metrics: Vec<MetricResult>,
    pub args: Vec<AnalysisArg>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricResult {
    pub name: String,
    pub phase: AnalysisPhase,
    pub measurements: Vec<Measurement>,
    pub failure_limit: u32,
    pub consecutive_errors: u32,
    pub count: u32,
    pub failed: u32,
    pub error: u32,
    pub inconclusive: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    pub value: Option<serde_json::Value>,
    pub phase: AnalysisPhase,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub message: Option<String>,
    pub resume_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AnalysisPhase {
    Pending,
    Running,
    Successful,
    Failed,
    Error,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRef {
    pub template_name: String,
    pub args: Vec<AnalysisArg>,
}

// ── Rollout Status ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutStatus {
    pub phase: RolloutPhase,
    pub message: Option<String>,
    pub current_step_index: Option<u32>,
    pub current_step_hash: Option<String>,
    pub canary_weight: u8,
    pub stable_rs: Option<String>,
    pub canary_rs: Option<String>,
    pub abort_after_step_index: Option<u32>,
    pub conditions: Vec<RolloutCondition>,
    pub canary: Option<CanaryStatus>,
    pub blue_green: Option<BlueGreenStatus>,
}

impl Default for RolloutStatus {
    fn default() -> Self {
        Self {
            phase: RolloutPhase::Pending,
            message: None,
            current_step_index: Some(0),
            current_step_hash: None,
            canary_weight: 0,
            stable_rs: None,
            canary_rs: None,
            abort_after_step_index: None,
            conditions: vec![],
            canary: None,
            blue_green: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RolloutPhase {
    Pending,
    Progressing,
    Paused,
    Healthy,
    Degraded,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutCondition {
    pub condition_type: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub last_update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryStatus {
    pub weights: TrafficWeights,
    pub current_step_analysis_run: Option<Uuid>,
    pub current_background_analysis_run: Option<Uuid>,
    pub current_step_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueGreenStatus {
    pub active_rs: Option<String>,
    pub preview_rs: Option<String>,
    pub pre_promotion_analysis_run: Option<Uuid>,
    pub post_promotion_analysis_run: Option<Uuid>,
    pub scale_down_delay_start_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficWeights {
    pub stable: WeightDestination,
    pub canary: WeightDestination,
    pub additional: Vec<WeightDestination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightDestination {
    pub service_name: String,
    pub weight: u8,
    pub pod_template_hash: Option<String>,
}

// ── Traffic configuration ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficConfig {
    pub provider: TrafficProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrafficProvider {
    /// cave-mesh (Envoy/Istio) virtual service
    CaveMesh {
        virtual_service_name: String,
        destination_rule_name: String,
    },
    /// Nginx ingress
    Nginx {
        ingress_name: String,
        service_port: u16,
        additional_ingress_annotations: Option<HashMap<String, String>>,
    },
    /// AWS ALB
    Alb {
        ingress_name: String,
        annotation_prefix: Option<String>,
    },
    /// SMI TrafficSplit
    Smi {
        root_service: String,
        traffic_split_name: String,
    },
}

// ── Notifications ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationTarget {
    Slack {
        webhook_url: String,
        channel: Option<String>,
    },
    Webhook {
        url: String,
        headers: Option<HashMap<String, String>>,
    },
    Teams {
        webhook_url: String,
    },
    PagerDuty {
        service_key: String,
    },
}

// ── API request/response types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateRolloutRequest {
    pub name: String,
    pub namespace: String,
    pub workload_ref: WorkloadRef,
    pub strategy: RolloutStrategy,
    pub traffic: Option<TrafficConfig>,
    pub analysis: Option<AnalysisRef>,
    pub notifications: Option<Vec<NotificationTarget>>,
}

#[derive(Debug, Deserialize)]
pub struct RolloutActionRequest {
    pub action: RolloutAction,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutAction {
    /// Promote the canary to the next step.
    Promote,
    /// Fully promote (skip all remaining steps).
    PromoteFull,
    /// Abort and rollback to stable.
    Abort,
    /// Pause an in-progress rollout.
    Pause,
    /// Resume a paused rollout.
    Resume,
    /// Retry a failed/aborted rollout.
    Retry,
}

#[derive(Debug, Deserialize)]
pub struct CreateAnalysisTemplateRequest {
    pub name: String,
    pub namespace: String,
    pub metrics: Vec<MetricSpec>,
    pub args: Vec<AnalysisArg>,
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub namespace: Option<String>,
}

//! Domain types for cave-rollouts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Rollout Phase ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RolloutPhase {
    /// Rollout has not started.
    Pending,
    /// Progressing through steps.
    Progressing,
    /// Paused, waiting for manual promotion or timer.
    Paused,
    /// All traffic on the new version; analysis passed.
    Healthy,
    /// Rolled back to stable version.
    Degraded,
    /// Permanent failure.
    Error,
}

impl RolloutPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, RolloutPhase::Healthy | RolloutPhase::Degraded | RolloutPhase::Error)
    }
}

// ─── Strategy ─────────────────────────────────────────────────────────────────

/// Canary step: either set weight, pause, set header route, or mirror.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RolloutStep {
    /// Send `weight`% of traffic to the canary.
    SetWeight { weight: u8 },
    /// Pause for `duration_seconds`, or indefinitely if None (requires manual promotion).
    Pause { duration_seconds: Option<u64> },
    /// Route traffic with a specific header to the canary.
    SetHeaderRoute {
        header_name: String,
        header_value: String,
    },
    /// Mirror `weight`% of requests to canary (responses discarded).
    SetMirrorRoute { weight: u8 },
    /// Run an analysis template inline as a step.
    Analysis { template_name: String },
}

/// Traffic-split thresholds for canary analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryStrategy {
    /// Ordered rollout steps.
    pub steps: Vec<RolloutStep>,
    /// Name of the AnalysisTemplate to run at each step.
    pub analysis_template: Option<String>,
    /// Maximum allowed error percentage before rollback.
    pub max_surge: u8,
}

impl CanaryStrategy {
    pub fn weight_at_step(&self, step_index: usize) -> Option<u8> {
        self.steps.get(step_index).and_then(|s| {
            if let RolloutStep::SetWeight { weight } = s {
                Some(*weight)
            } else {
                None
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueGreenStrategy {
    /// Name of the service pointing at the active (stable) version.
    pub active_service: String,
    /// Name of the service pointing at the preview (new) version.
    pub preview_service: String,
    /// Seconds to wait after switching before marking healthy.
    pub auto_promotion_seconds: Option<u64>,
    pub analysis_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbTestingStrategy {
    pub steps: Vec<RolloutStep>,
    pub analysis_template: Option<String>,
    /// HTTP header used to route users to variants.
    pub routing_header: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum RolloutStrategy {
    Canary(CanaryStrategy),
    BlueGreen(BlueGreenStrategy),
    AbTesting(AbTestingStrategy),
}

// ─── Rollout ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rollout {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub strategy: RolloutStrategy,
    pub phase: RolloutPhase,
    /// Index of the current step being executed.
    pub current_step_index: usize,
    /// The stable (baseline) image/revision.
    pub stable_revision: String,
    /// The canary/new image/revision.
    pub canary_revision: String,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Rollout {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        strategy: RolloutStrategy,
        stable_revision: impl Into<String>,
        canary_revision: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            strategy,
            phase: RolloutPhase::Pending,
            current_step_index: 0,
            stable_revision: stable_revision.into(),
            canary_revision: canary_revision.into(),
            message: None,
            created_at: now,
            updated_at: now,
        }
    }
}

// ─── Analysis ─────────────────────────────────────────────────────────────────

/// A single metric in an analysis template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricTemplate {
    pub name: String,
    pub provider: MetricProvider,
    /// PromQL / HTTP query.
    pub query: String,
    /// Minimum value to consider successful (inclusive).
    pub success_condition: MetricCondition,
    /// Minimum consecutive failures before marking the metric as failed.
    pub failure_limit: u32,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricProvider {
    /// cave-metrics Prometheus endpoint.
    CaveMetrics { address: String },
    /// Arbitrary HTTP webhook returning `{"value": <f64>}`.
    Webhook { url: String },
}

/// Threshold condition for a metric value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MetricCondition {
    GreaterThan { threshold: f64 },
    LessThan { threshold: f64 },
    Between { lo: f64, hi: f64 },
}

impl MetricCondition {
    pub fn evaluate(&self, value: f64) -> bool {
        match self {
            MetricCondition::GreaterThan { threshold } => value > *threshold,
            MetricCondition::LessThan { threshold } => value < *threshold,
            MetricCondition::Between { lo, hi } => value >= *lo && value <= *hi,
        }
    }
}

/// A reusable analysis template (analogous to Argo's AnalysisTemplate CRD).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisTemplate {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub metrics: Vec<MetricTemplate>,
    pub created_at: DateTime<Utc>,
}

impl AnalysisTemplate {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        metrics: Vec<MetricTemplate>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            metrics,
            created_at: Utc::now(),
        }
    }
}

// ─── Analysis Run ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
pub struct MetricResult {
    pub metric_name: String,
    pub value: f64,
    pub passed: bool,
    pub failure_count: u32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRun {
    pub id: Uuid,
    pub rollout_id: Option<Uuid>,
    pub template_name: String,
    pub phase: AnalysisPhase,
    pub metric_results: Vec<MetricResult>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl AnalysisRun {
    pub fn new(template_name: impl Into<String>, rollout_id: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            rollout_id,
            template_name: template_name.into(),
            phase: AnalysisPhase::Pending,
            metric_results: Vec::new(),
            start_time: None,
            end_time: None,
            created_at: Utc::now(),
        }
    }

    /// Overall success: all metrics passed and none exceeded failure limit.
    pub fn is_successful(&self) -> bool {
        !self.metric_results.is_empty() && self.metric_results.iter().all(|r| r.passed)
    }
}

// ─── Experiment ───────────────────────────────────────────────────────────────

/// An A/B experiment variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentVariant {
    pub name: String,
    pub template_name: String,
    pub replicas: u32,
    /// Traffic weight for this variant.
    pub weight: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ExperimentPhase {
    Pending,
    Running,
    Successful,
    Failed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub variants: Vec<ExperimentVariant>,
    pub analysis_templates: Vec<String>,
    pub duration_seconds: Option<u64>,
    pub phase: ExperimentPhase,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Experiment {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        variants: Vec<ExperimentVariant>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            variants,
            analysis_templates: Vec::new(),
            duration_seconds: None,
            phase: ExperimentPhase::Pending,
            start_time: None,
            end_time: None,
            created_at: Utc::now(),
        }
    }

    /// Total weight across all variants; should sum to 100.
    pub fn total_weight(&self) -> u32 {
        self.variants.iter().map(|v| v.weight as u32).sum()
    }
}

// ─── Notifications ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEvent {
    RolloutStarted,
    RolloutPromoted,
    RolloutRolledBack,
    RolloutPaused,
    AnalysisCompleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationChannel {
    Webhook { url: String },
    Slack {
        webhook_url: String,
        channel: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub channels: Vec<NotificationChannel>,
    pub events: Vec<NotificationEvent>,
    pub labels: HashMap<String, String>,
}

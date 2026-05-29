// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DevLake data models — DORA metrics, pipelines, deployments, incidents, PRs, issues, sprints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── DORA Rating ───────────────────────────────────────────────────────────────

/// DORA performance band.  Ordered Elite > High > Medium > Low.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DoraRating {
    Low,
    Medium,
    High,
    Elite,
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStatus {
    Pending,
    Running,
    InProgress,
    Success,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineStage {
    pub name: String,
    pub status: PipelineStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<f64>,
    pub logs_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pipeline {
    pub id: Uuid,
    pub name: String,
    pub project: String,
    pub repo: String,
    pub branch: String,
    pub status: PipelineStatus,
    pub triggered_by: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<f64>,
    pub stages: Vec<PipelineStage>,
    pub commit_sha: Option<String>,
    pub environment: DeploymentEnv,
}

// ── Deployment ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentEnv {
    Development,
    Testing,
    Staging,
    Production,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    Running,
    Success,
    Failed,
    RolledBack,
    Cancelled,
}

/// A single deployment event tracked for DORA metrics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Deployment {
    pub id: Uuid,
    pub pipeline_id: Option<Uuid>,
    pub service: String,
    pub version: String,
    pub environment: DeploymentEnv,
    pub deployed_at: DateTime<Utc>,
    pub deployed_by: String,
    pub status: DeploymentStatus,
    pub rollback: bool,
    /// Elapsed seconds from code-commit to production deployment.
    pub lead_time_secs: Option<f64>,
}

// ── Incident ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Incident {
    pub id: Uuid,
    pub title: String,
    pub severity: String,
    pub started_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub services: Vec<String>,
    pub linked_deployment_id: Option<Uuid>,
}

impl Incident {
    /// MTTR for this incident in seconds, if resolved.
    pub fn mttr_secs(&self) -> Option<f64> {
        self.resolved_at
            .map(|r| (r - self.started_at).num_seconds() as f64)
    }
}

// ── Pull Request ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Closed,
    Draft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PullRequest {
    pub id: Uuid,
    pub number: u64,
    pub title: String,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub state: PrState,
    pub created_at: DateTime<Utc>,
    pub merged_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    /// Time from PR creation to merge/close in seconds.
    pub cycle_time_secs: Option<f64>,
    pub review_count: u32,
    pub comment_count: u32,
    pub additions: u32,
    pub deletions: u32,
}

impl PullRequest {
    /// Compute cycle time from created_at to merged_at.
    pub fn compute_cycle_time(&self) -> Option<f64> {
        self.merged_at
            .map(|m| (m - self.created_at).num_seconds() as f64)
    }
}

// ── Commit ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Commit {
    pub sha: String,
    pub author: String,
    pub message: String,
    pub committed_at: DateTime<Utc>,
    pub additions: u32,
    pub deletions: u32,
    pub files_changed: u32,
}

// ── Issue (Jira-like) ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Open,
    InProgress,
    Done,
    Closed,
    Backlog,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    Bug,
    Story,
    Task,
    Epic,
    SubTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Issue {
    pub id: Uuid,
    pub key: String,
    pub title: String,
    pub issue_type: IssueType,
    pub status: IssueStatus,
    pub assignee: Option<String>,
    pub priority: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub story_points: Option<u32>,
}

impl Issue {
    /// Resolution time in seconds, if resolved.
    pub fn resolution_time_secs(&self) -> Option<f64> {
        self.resolved_at
            .map(|r| (r - self.created_at).num_seconds() as f64)
    }
}

// ── Sprint ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SprintState {
    Future,
    Active,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sprint {
    pub id: Uuid,
    pub name: String,
    pub state: SprintState,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    pub completed_points: u32,
    pub planned_points: u32,
    pub completed_issues: u32,
    pub planned_issues: u32,
}

impl Sprint {
    /// Velocity as percentage of planned points completed.
    pub fn velocity_pct(&self) -> f64 {
        if self.planned_points == 0 {
            return 0.0;
        }
        self.completed_points as f64 / self.planned_points as f64 * 100.0
    }
}

// ── DORA Report ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoraReport {
    pub period_days: u32,
    pub deployment_frequency_per_day: f64,
    pub deployment_frequency_rating: DoraRating,
    pub lead_time_secs: f64,
    pub lead_time_rating: DoraRating,
    pub change_failure_rate_pct: f64,
    pub change_failure_rate_rating: DoraRating,
    pub mttr_secs: f64,
    pub mttr_rating: DoraRating,
    pub overall_rating: DoraRating,
}

// ── Legacy compatibility types ────────────────────────────────────────────────
// These were in the original models.rs; kept for engine.rs compatibility.

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

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Data models for cave-gitops-config.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Promise ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromiseStatus {
    Active,
    Deprecated,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStageType {
    /// Modify the resource manifest.
    Transform,
    /// Add default configurations.
    Configure,
    /// Write to the state store.
    Deploy,
    /// Check constraints on the spec.
    Validate,
    /// Log a notification.
    Notify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    pub name: String,
    pub description: String,
    pub stage_type: PipelineStageType,
    pub config: serde_json::Value,
    pub order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestinationSelector {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Promise {
    pub id: Uuid,
    /// Unique short name (e.g., "postgresql", "redis").
    pub name: String,
    pub version: String,
    pub description: String,
    /// JSON Schema for validating resource requests.
    pub api_schema: serde_json::Value,
    pub pipeline: Vec<PipelineStage>,
    /// Names of other promises this one depends on.
    pub dependencies: Vec<String>,
    pub destination_selectors: Vec<DestinationSelector>,
    pub status: PromiseStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Resource Request ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceRequestStatus {
    Pending,
    InPipeline,
    Ready,
    Failed,
    Deleting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStageResult {
    pub stage_name: String,
    pub status: StageStatus,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: Uuid,
    pub resource_request_id: Uuid,
    pub promise_name: String,
    pub stages: Vec<PipelineStageResult>,
    pub status: PipelineRunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub id: Uuid,
    pub promise_name: String,
    pub promise_version: String,
    pub namespace: String,
    pub name: String,
    /// Must conform to the promise's api_schema.
    pub spec: serde_json::Value,
    pub requester: Uuid,
    pub status: ResourceRequestStatus,
    pub pipeline_run: Option<PipelineRun>,
    /// Cluster names where the resource was deployed.
    pub destinations: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── State Store ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Unknown,
    Error,
}

/// Compare desired vs live resource state and compute the [`SyncStatus`].
///
/// Ported from ArgoCD `controller/state.go` `CompareAppState`: the controller
/// diffs the rendered desired manifest against the observed live manifest. When
/// every desired resource has a matching live counterpart with no diff, the app
/// is `Synced`; any drift, or a desired resource with no live target, is
/// `OutOfSync`.
///
/// Manifests are normalized before comparison (trailing whitespace / blank
/// trailing lines stripped) mirroring ArgoCD's normalization step, so cosmetic
/// formatting differences do not produce false drift.
///
/// `live == None` represents a desired resource that has no observed live
/// target (e.g. not yet applied or pruned) which ArgoCD reports as `OutOfSync`.
pub fn compare_state(desired: &str, live: Option<&str>) -> SyncStatus {
    match live {
        None => SyncStatus::OutOfSync,
        Some(live) => {
            if normalize_manifest(desired) == normalize_manifest(live) {
                SyncStatus::Synced
            } else {
                SyncStatus::OutOfSync
            }
        }
    }
}

/// Normalize a manifest for diffing: strip trailing whitespace from each line
/// and drop trailing blank lines. Mirrors ArgoCD's pre-diff normalization which
/// canonicalizes formatting before comparing desired vs live.
fn normalize_manifest(manifest: &str) -> String {
    let mut lines: Vec<&str> = manifest.lines().map(|l| l.trim_end()).collect();
    while matches!(lines.last(), Some(&"")) {
        lines.pop();
    }
    lines.join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStoreEntry {
    pub id: Uuid,
    /// e.g. "clusters/prod/postgresql/default/my-db.yaml"
    pub path: String,
    pub cluster: String,
    /// YAML content
    pub content: String,
    pub checksum: String,
    pub promise_name: String,
    pub resource_request_id: Uuid,
    pub last_synced: Option<DateTime<Utc>>,
    pub sync_status: SyncStatus,
}

// ─── Cluster ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterStatus {
    Ready,
    NotReady,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterDestination {
    pub name: String,
    pub api_server: String,
    pub labels: HashMap<String, String>,
    pub status: ClusterStatus,
    pub registered_at: DateTime<Utc>,
}

// ─── Request Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePromiseRequest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub api_schema: serde_json::Value,
    pub pipeline: Vec<PipelineStage>,
    pub dependencies: Option<Vec<String>>,
    pub destination_selectors: Option<Vec<DestinationSelector>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateResourceRequestRequest {
    pub promise_name: String,
    pub promise_version: String,
    pub namespace: String,
    pub name: String,
    pub spec: serde_json::Value,
    pub requester: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterClusterRequest {
    pub name: String,
    pub api_server: String,
    pub labels: Option<HashMap<String, String>>,
}

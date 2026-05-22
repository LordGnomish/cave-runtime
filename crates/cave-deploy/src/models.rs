// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ArgoCD/Flux-compatible CRD models.
//!
//! Covers: Application, ApplicationSet, AppProject, sync policy,
//! health status, resource tracking, notifications.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Source types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSource {
    pub repo_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Helm-specific configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helm: Option<HelmSource>,
    /// Kustomize-specific configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kustomize: Option<KustomizeSource>,
    /// Plain directory (no tool).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<DirectorySource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelmSource {
    /// Extra values files relative to the chart root.
    #[serde(default)]
    pub value_files: Vec<String>,
    /// Inline value overrides.
    #[serde(default)]
    pub values: String,
    /// --set KEY=VALUE overrides.
    #[serde(default)]
    pub parameters: Vec<HelmParameter>,
    /// --set-file KEY=PATH overrides.
    #[serde(default)]
    pub file_parameters: Vec<HelmFileParameter>,
    /// Chart release name (defaults to app name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_name: Option<String>,
    /// Specific chart name within a Helm repo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart: Option<String>,
    /// Skip CRDs during sync.
    #[serde(default)]
    pub skip_crds: bool,
    /// Pass credentials to all Helm repos.
    #[serde(default)]
    pub pass_credentials: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmParameter {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub force_string: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmFileParameter {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KustomizeSource {
    /// Kustomize version to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Image overrides: image_name=new_image:tag.
    #[serde(default)]
    pub images: Vec<String>,
    /// Name prefix for all resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_prefix: Option<String>,
    /// Name suffix for all resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_suffix: Option<String>,
    /// Additional common labels.
    #[serde(default)]
    pub common_labels: HashMap<String, String>,
    /// Additional common annotations.
    #[serde(default)]
    pub common_annotations: HashMap<String, String>,
    /// Build patches.
    #[serde(default)]
    pub patches: Vec<KustomizePatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KustomizePatch {
    pub target: Option<KustomizePatchTarget>,
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KustomizePatchTarget {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectorySource {
    #[serde(default)]
    pub recurse: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jsonnet: Option<JsonnetConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonnetConfig {
    pub libs: Vec<String>,
    pub tlas: Vec<JsonnetVar>,
    pub ext_vars: Vec<JsonnetVar>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonnetVar {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub code: bool,
}

// ─── Destination ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Destination {
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub namespace: String,
}

// ─── Sync policy ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncPolicy {
    /// Automated sync configuration (ArgoCD automatedPolicy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automated: Option<AutomatedSyncPolicy>,
    /// Sync options (--sync-option flags).
    #[serde(default)]
    pub sync_options: Vec<String>,
    /// Retry on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<SyncRetry>,
    /// Managed namespace configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_namespace_metadata: Option<ManagedNamespaceMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomatedSyncPolicy {
    /// Delete resources that are no longer in git.
    #[serde(default)]
    pub prune: bool,
    /// Revert manual changes in cluster.
    #[serde(default)]
    pub self_heal: bool,
    /// Allow empty sync (no resources).
    #[serde(default)]
    pub allow_empty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRetry {
    pub limit: i64,
    pub backoff: Option<SyncRetryBackoff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRetryBackoff {
    pub duration: String,
    pub factor: Option<i64>,
    pub max_duration: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedNamespaceMetadata {
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
}

// ─── Sync waves and hooks ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum SyncPhase {
    PreSync,
    Sync,
    PostSync,
    SyncFail,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceHook {
    pub name: String,
    pub kind: String,
    pub api_version: String,
    pub namespace: String,
    pub phases: Vec<SyncPhase>,
    /// Delete after successful sync.
    #[serde(default)]
    pub delete_on_success: bool,
    /// Wave within sync phase (lower = earlier).
    #[serde(default)]
    pub wave: i32,
}

// ─── Health status ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum HealthStatus {
    Healthy,
    Progressing,
    Degraded,
    Suspended,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCondition {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ─── Sync status ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Unknown,
}

// ─── Application status ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationStatus {
    pub health: HealthCondition,
    pub sync: SyncCondition,
    pub resources: Vec<ResourceStatus>,
    pub history: Vec<RevisionHistory>,
    pub conditions: Vec<ApplicationCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
    pub reconciled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCondition {
    pub status: SyncStatus,
    pub revision: String,
    pub revisions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStatus {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub status: SyncStatus,
    pub health: Option<HealthCondition>,
    pub hook: bool,
    pub require_pruning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionHistory {
    pub id: u64,
    pub revision: String,
    pub deployed_at: DateTime<Utc>,
    pub initiated_by: String,
    pub source: ApplicationSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationCondition {
    pub condition_type: String,
    pub message: String,
    pub last_transition_time: DateTime<Utc>,
}

// ─── Resource tracking ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceTracking {
    Annotation,
    Label,
    /// ArgoCD tracking label (default).
    ArgocdLabel,
}

impl Default for ResourceTracking {
    fn default() -> Self {
        Self::ArgocdLabel
    }
}

// ─── Application CRD ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSpec {
    pub source: ApplicationSource,
    /// Multi-source: additional sources merged in.
    #[serde(default)]
    pub sources: Vec<ApplicationSource>,
    pub destination: Destination,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_policy: Option<SyncPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignored_differences: Option<Vec<IgnoredDifference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<Vec<ApplicationInfo>>,
    #[serde(default)]
    pub revision_history_limit: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredDifference {
    pub group: Option<String>,
    pub kind: String,
    pub name: Option<String>,
    pub namespace: Option<String>,
    #[serde(default)]
    pub json_pointers: Vec<String>,
    #[serde(default)]
    pub jq_path_expressions: Vec<String>,
    #[serde(default)]
    pub managed_fields_managers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationInfo {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub spec: ApplicationSpec,
    pub status: Option<ApplicationStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    pub tracking: ResourceTracking,
}

// ─── Sync operation ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOperation {
    pub id: Uuid,
    pub application_id: Uuid,
    pub revision: String,
    pub dry_run: bool,
    pub prune: bool,
    pub force: bool,
    pub resources: Option<Vec<SyncResourceFilter>>,
    pub initiated_by: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub phase: SyncOperationPhase,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum SyncOperationPhase {
    Running,
    Failed,
    Succeeded,
    Error,
    Terminating,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResourceFilter {
    pub group: String,
    pub kind: String,
    pub name: String,
}

// ─── Repository credentials ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCredential {
    pub id: Uuid,
    pub url: String,
    pub credential_type: CredentialType,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CredentialType {
    SshKey {
        private_key_ref: String,
    },
    HttpsPassword {
        username: String,
        password_ref: String,
    },
    GithubApp {
        app_id: u64,
        installation_id: u64,
        private_key_ref: String,
    },
    GcpServiceAccount {
        service_account_ref: String,
    },
}

// ─── Notifications ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationConfig {
    pub id: Uuid,
    pub name: String,
    pub triggers: Vec<NotificationTrigger>,
    pub destination: NotificationDestination,
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum NotificationTrigger {
    OnSyncSucceeded,
    OnSyncFailed,
    OnHealthDegraded,
    OnDeployed,
    OnSyncRunning,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum NotificationDestination {
    Slack { channel: String },
    Email { addresses: Vec<String> },
    Webhook { url: String },
    MSTeams { webhook_url: String },
    PagerDuty { routing_key_ref: String },
}

// ─── SSO ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SSOConfig {
    pub provider: SSOProvider,
    pub client_id: String,
    pub client_secret_ref: String,
    pub issuer: Option<String>,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub groups_claim: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SSOProvider {
    Dex,
    OIDC,
    GitHub,
    GitLab,
    Google,
    Microsoft,
    Okta,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_source_helm_roundtrip() {
        let src = ApplicationSource {
            repo_url: "https://charts.example.com".to_string(),
            target_revision: Some("1.2.3".to_string()),
            path: None,
            helm: Some(HelmSource {
                value_files: vec!["values-prod.yaml".to_string()],
                values: "replicaCount: 3".to_string(),
                parameters: vec![HelmParameter {
                    name: "image.tag".to_string(),
                    value: "v1.0.0".to_string(),
                    force_string: true,
                }],
                file_parameters: vec![],
                release_name: Some("my-app".to_string()),
                chart: Some("my-chart".to_string()),
                skip_crds: false,
                pass_credentials: false,
            }),
            kustomize: None,
            directory: None,
        };
        let json = serde_json::to_string(&src).unwrap();
        let back: ApplicationSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back.repo_url, "https://charts.example.com");
        assert!(back.helm.is_some());
    }

    #[test]
    fn sync_policy_automated() {
        let policy = SyncPolicy {
            automated: Some(AutomatedSyncPolicy {
                prune: true,
                self_heal: true,
                allow_empty: false,
            }),
            sync_options: vec!["CreateNamespace=true".to_string()],
            retry: Some(SyncRetry {
                limit: 5,
                backoff: Some(SyncRetryBackoff {
                    duration: "5s".to_string(),
                    factor: Some(2),
                    max_duration: Some("3m".to_string()),
                }),
            }),
            managed_namespace_metadata: None,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: SyncPolicy = serde_json::from_str(&json).unwrap();
        assert!(back.automated.unwrap().prune);
        assert_eq!(back.sync_options[0], "CreateNamespace=true");
    }

    #[test]
    fn resource_tracking_default() {
        assert_eq!(ResourceTracking::default(), ResourceTracking::ArgocdLabel);
    }

    #[test]
    fn health_status_variants() {
        let healthy = HealthStatus::Healthy;
        let json = serde_json::to_string(&healthy).unwrap();
        assert_eq!(json, "\"Healthy\"");
        let degraded: HealthStatus = serde_json::from_str("\"Degraded\"").unwrap();
        assert_eq!(degraded, HealthStatus::Degraded);
    }

    #[test]
    fn sync_phase_presync() {
        let phase = SyncPhase::PreSync;
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(json, "\"PreSync\"");
    }

    #[test]
    fn application_spec_multi_source() {
        let spec = ApplicationSpec {
            source: ApplicationSource {
                repo_url: "https://github.com/example/app".to_string(),
                target_revision: Some("main".to_string()),
                path: Some("k8s/".to_string()),
                helm: None,
                kustomize: None,
                directory: None,
            },
            sources: vec![ApplicationSource {
                repo_url: "https://github.com/example/config".to_string(),
                target_revision: Some("main".to_string()),
                path: Some("values/".to_string()),
                helm: None,
                kustomize: None,
                directory: None,
            }],
            destination: Destination {
                server: "https://kubernetes.default.svc".to_string(),
                name: None,
                namespace: "production".to_string(),
            },
            project: "default".to_string(),
            sync_policy: None,
            ignored_differences: None,
            info: None,
            revision_history_limit: Some(10),
        };
        assert_eq!(spec.sources.len(), 1);
        assert_eq!(spec.destination.namespace, "production");
    }
}

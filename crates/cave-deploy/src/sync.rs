// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sync engine — manual, auto, self-heal, prune, dry-run, waves, hooks.

use crate::models::*;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ─── Sync request ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub application_id: Uuid,
    pub revision: Option<String>,
    pub dry_run: bool,
    pub prune: bool,
    pub force: bool,
    pub resource_filter: Option<Vec<SyncResourceFilter>>,
    pub strategy: SyncStrategy,
    pub initiated_by: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncStrategy {
    Apply,
    Hook,
}

impl Default for SyncStrategy {
    fn default() -> Self {
        Self::Apply
    }
}

/// Outcome of a single resource sync attempt.
#[derive(Debug, Clone)]
pub struct ResourceSyncResult {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub action: SyncAction,
    pub status: ResourceSyncStatus,
    pub message: Option<String>,
    pub hook_phase: Option<SyncPhase>,
    pub wave: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncAction {
    Create,
    Update,
    Delete,
    Skip,
    Hook,
    Replace,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResourceSyncStatus {
    Succeeded,
    Failed,
    Running,
    Pruned,
    Skipped,
}

// ─── Sync wave ordering ──────────────────────────────────────────────────────

/// Group resources by their sync wave annotation.
/// Resources with lower wave values are applied first.
pub fn group_by_wave(resources: &[ManifestResource]) -> Vec<(i32, Vec<&ManifestResource>)> {
    let mut wave_map: HashMap<i32, Vec<&ManifestResource>> = HashMap::new();
    for res in resources {
        let wave = res.wave;
        wave_map.entry(wave).or_default().push(res);
    }
    let mut waves: Vec<(i32, Vec<&ManifestResource>)> = wave_map.into_iter().collect();
    waves.sort_by_key(|(w, _)| *w);
    waves
}

/// Resource from a rendered manifest.
#[derive(Debug, Clone)]
pub struct ManifestResource {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub wave: i32,
    pub hook_phases: Vec<SyncPhase>,
    pub delete_on_success: bool,
    pub manifest: serde_json::Value,
}

impl ManifestResource {
    pub fn is_hook(&self) -> bool {
        !self.hook_phases.is_empty()
    }

    pub fn is_in_phase(&self, phase: &SyncPhase) -> bool {
        self.hook_phases.contains(phase)
    }
}

/// Parse sync wave from ArgoCD annotations.
pub fn parse_wave(annotations: &HashMap<String, String>) -> i32 {
    annotations
        .get("argocd.argoproj.io/sync-wave")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Parse hook phases from ArgoCD hook annotation.
pub fn parse_hook_phases(annotations: &HashMap<String, String>) -> Vec<SyncPhase> {
    annotations
        .get("argocd.argoproj.io/hook")
        .map(|s| {
            s.split(',').filter_map(|p| match p.trim() {
                "PreSync" => Some(SyncPhase::PreSync),
                "Sync" => Some(SyncPhase::Sync),
                "PostSync" => Some(SyncPhase::PostSync),
                "SyncFail" => Some(SyncPhase::SyncFail),
                "Skip" => Some(SyncPhase::Skip),
                _ => None,
            }).collect()
        })
        .unwrap_or_default()
}

/// Parse hook deletion policy.
pub fn parse_delete_on_success(annotations: &HashMap<String, String>) -> bool {
    annotations
        .get("argocd.argoproj.io/hook-delete-policy")
        .map(|s| s.contains("HookSucceeded") || s.contains("BeforeHookCreation"))
        .unwrap_or(false)
}

// ─── Rollback ────────────────────────────────────────────────────────────────

/// Rollback request — target a specific revision history entry.
#[derive(Debug, Clone)]
pub struct RollbackRequest {
    pub application_id: Uuid,
    pub history_id: u64,
    pub prune: bool,
    pub dry_run: bool,
    pub initiated_by: String,
}

/// Result of a rollback initiation.
#[derive(Debug, Clone)]
pub struct RollbackResult {
    pub operation_id: Uuid,
    pub application_id: Uuid,
    pub target_revision: String,
    pub target_history_id: u64,
    pub started_at: chrono::DateTime<Utc>,
}

pub fn initiate_rollback(req: &RollbackRequest, history: &[RevisionHistory]) -> Option<RollbackResult> {
    let entry = history.iter().find(|h| h.id == req.history_id)?;
    Some(RollbackResult {
        operation_id: Uuid::new_v4(),
        application_id: req.application_id,
        target_revision: entry.revision.clone(),
        target_history_id: req.history_id,
        started_at: Utc::now(),
    })
}

// ─── Auto-sync evaluation ────────────────────────────────────────────────────

/// Determine whether auto-sync should trigger based on policy and current status.
pub fn should_auto_sync(
    policy: &SyncPolicy,
    current_sync: &SyncStatus,
    health: &HealthStatus,
) -> bool {
    let automated = match &policy.automated {
        Some(a) => a,
        None => return false,
    };

    match current_sync {
        SyncStatus::OutOfSync => true,
        SyncStatus::Synced => {
            // Self-heal: re-sync if app is synced but health has degraded
            automated.self_heal && *health == HealthStatus::Degraded
        }
        SyncStatus::Unknown => false,
    }
}

/// Determine whether pruning should occur for a resource.
pub fn should_prune(
    policy: &SyncPolicy,
    resource: &ResourceStatus,
) -> bool {
    let prune = policy.automated.as_ref().map(|a| a.prune).unwrap_or(false);
    prune && resource.require_pruning
}

// ─── Sync options parsing ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ParsedSyncOptions {
    pub create_namespace: bool,
    pub prune_last: bool,
    pub replace: bool,
    pub apply_out_of_sync_only: bool,
    pub server_side_apply: bool,
    pub fail_on_shared_resource: bool,
    pub respect_ignore_differences: bool,
    pub validate: bool,
}

pub fn parse_sync_options(options: &[String]) -> ParsedSyncOptions {
    let mut parsed = ParsedSyncOptions { validate: true, ..Default::default() };
    for opt in options {
        match opt.as_str() {
            "CreateNamespace=true" => parsed.create_namespace = true,
            "PruneLast=true" => parsed.prune_last = true,
            "Replace=true" => parsed.replace = true,
            "ApplyOutOfSyncOnly=true" => parsed.apply_out_of_sync_only = true,
            "ServerSideApply=true" => parsed.server_side_apply = true,
            "FailOnSharedResource=true" => parsed.fail_on_shared_resource = true,
            "RespectIgnoreDifferences=true" => parsed.respect_ignore_differences = true,
            "Validate=false" => parsed.validate = false,
            _ => {}
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{HealthStatus, SyncStatus, SyncPolicy, AutomatedSyncPolicy};

    #[test]
    fn parse_wave_from_annotations() {
        let mut annotations = HashMap::new();
        annotations.insert("argocd.argoproj.io/sync-wave".to_string(), "5".to_string());
        assert_eq!(parse_wave(&annotations), 5);
    }

    #[test]
    fn parse_wave_default_zero() {
        let annotations = HashMap::new();
        assert_eq!(parse_wave(&annotations), 0);
    }

    #[test]
    fn parse_hook_phases_presync() {
        let mut annotations = HashMap::new();
        annotations.insert("argocd.argoproj.io/hook".to_string(), "PreSync".to_string());
        let phases = parse_hook_phases(&annotations);
        assert_eq!(phases, vec![SyncPhase::PreSync]);
    }

    #[test]
    fn parse_hook_phases_multiple() {
        let mut annotations = HashMap::new();
        annotations.insert("argocd.argoproj.io/hook".to_string(), "PreSync,Sync".to_string());
        let phases = parse_hook_phases(&annotations);
        assert_eq!(phases.len(), 2);
    }

    #[test]
    fn group_by_wave_ordering() {
        let resources = vec![
            ManifestResource { group: "".to_string(), version: "v1".to_string(), kind: "ConfigMap".to_string(), namespace: "default".to_string(), name: "cm".to_string(), wave: 2, hook_phases: vec![], delete_on_success: false, manifest: serde_json::json!({}) },
            ManifestResource { group: "apps".to_string(), version: "v1".to_string(), kind: "Deployment".to_string(), namespace: "default".to_string(), name: "dep".to_string(), wave: 0, hook_phases: vec![], delete_on_success: false, manifest: serde_json::json!({}) },
            ManifestResource { group: "".to_string(), version: "v1".to_string(), kind: "Service".to_string(), namespace: "default".to_string(), name: "svc".to_string(), wave: 1, hook_phases: vec![], delete_on_success: false, manifest: serde_json::json!({}) },
        ];
        let waves = group_by_wave(&resources);
        assert_eq!(waves[0].0, 0); // Deployment first
        assert_eq!(waves[1].0, 1); // Service
        assert_eq!(waves[2].0, 2); // ConfigMap last
    }

    #[test]
    fn should_auto_sync_out_of_sync() {
        let policy = SyncPolicy {
            automated: Some(AutomatedSyncPolicy { prune: false, self_heal: false, allow_empty: false }),
            sync_options: vec![],
            retry: None,
            managed_namespace_metadata: None,
        };
        assert!(should_auto_sync(&policy, &SyncStatus::OutOfSync, &HealthStatus::Healthy));
    }

    #[test]
    fn should_auto_sync_self_heal_degraded() {
        let policy = SyncPolicy {
            automated: Some(AutomatedSyncPolicy { prune: false, self_heal: true, allow_empty: false }),
            sync_options: vec![],
            retry: None,
            managed_namespace_metadata: None,
        };
        assert!(should_auto_sync(&policy, &SyncStatus::Synced, &HealthStatus::Degraded));
        assert!(!should_auto_sync(&policy, &SyncStatus::Synced, &HealthStatus::Healthy));
    }

    #[test]
    fn should_auto_sync_no_policy() {
        let policy = SyncPolicy::default();
        assert!(!should_auto_sync(&policy, &SyncStatus::OutOfSync, &HealthStatus::Healthy));
    }

    #[test]
    fn parse_sync_options_create_namespace() {
        let opts = vec!["CreateNamespace=true".to_string(), "ServerSideApply=true".to_string()];
        let parsed = parse_sync_options(&opts);
        assert!(parsed.create_namespace);
        assert!(parsed.server_side_apply);
        assert!(parsed.validate);
    }

    #[test]
    fn parse_sync_options_validate_false() {
        let opts = vec!["Validate=false".to_string()];
        let parsed = parse_sync_options(&opts);
        assert!(!parsed.validate);
    }

    #[test]
    fn rollback_finds_history_entry() {
        let app_id = Uuid::new_v4();
        let history = vec![
            RevisionHistory {
                id: 1,
                revision: "abc123".to_string(),
                deployed_at: Utc::now(),
                initiated_by: "user".to_string(),
                source: ApplicationSource {
                    repo_url: "https://github.com/example/app".to_string(),
                    target_revision: Some("abc123".to_string()),
                    path: None,
                    helm: None,
                    kustomize: None,
                    directory: None,
                },
            },
        ];
        let req = RollbackRequest {
            application_id: app_id,
            history_id: 1,
            prune: false,
            dry_run: false,
            initiated_by: "user".to_string(),
        };
        let result = initiate_rollback(&req, &history).unwrap();
        assert_eq!(result.target_revision, "abc123");
    }

    #[test]
    fn rollback_not_found() {
        let req = RollbackRequest {
            application_id: Uuid::new_v4(),
            history_id: 99,
            prune: false,
            dry_run: false,
            initiated_by: "user".to_string(),
        };
        assert!(initiate_rollback(&req, &[]).is_none());
    }
}

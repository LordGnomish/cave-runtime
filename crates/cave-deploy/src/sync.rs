//! Sync engine — git pull, manifest parsing, wave ordering, hook execution,
//! drift detection, auto-sync loop, retry with backoff, and rollback.
//!
//! Annotation constants mirror ArgoCD so existing manifests work unchanged.

use crate::diff::{compute_diff, is_out_of_sync};
use crate::error::DeployError;
use crate::health::assess_resource_health;
use crate::models::{
    Application, DiffType, HealthStatusDetail, Manifest, OperationPhase, OperationState,
    ResourceDiff, ResourceResult, ResourceStatus, ResourceSyncStatus, RevisionHistory,
    SyncHookType, SyncOperationResult, SyncRequest, SyncStatus, SyncStatusDetail,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tracing::{debug, info, warn};
use uuid::Uuid;

// ─── ArgoCD-compatible annotation keys ────────────────────────────────────────

pub const ANNOTATION_SYNC_WAVE: &str = "argocd.argoproj.io/sync-wave";
pub const ANNOTATION_HOOK: &str = "argocd.argoproj.io/hook";
pub const ANNOTATION_HOOK_DELETE_POLICY: &str = "argocd.argoproj.io/hook-delete-policy";
pub const LABEL_MANAGED_BY: &str = "app.kubernetes.io/managed-by";
pub const LABEL_APP_NAME: &str = "argocd.argoproj.io/app-name";
pub const CAVE_MANAGER: &str = "cave-deploy";

// ─── Git operations ────────────────────────────────────────────────────────────

/// Manages a local git clone for one repository.
pub struct GitRepo {
    pub repo_url: String,
    pub work_dir: PathBuf,
}

impl GitRepo {
    pub fn new(repo_url: &str, base_dir: &Path) -> Self {
        // Derive a safe dir name from the URL
        let safe = repo_url
            .replace("://", "_")
            .replace('/', "_")
            .replace(':', "_");
        let work_dir = base_dir.join(&safe);
        Self { repo_url: repo_url.to_string(), work_dir }
    }

    /// Clone the repo if needed, then fetch and checkout the given revision.
    /// Returns the resolved commit SHA.
    pub async fn fetch(&self, revision: &str) -> Result<String, DeployError> {
        if self.work_dir.exists() {
            // Already cloned — fetch latest
            let fetch = Command::new("git")
                .args(["fetch", "--all", "--tags"])
                .current_dir(&self.work_dir)
                .output()
                .await?;
            if !fetch.status.success() {
                warn!(
                    url = %self.repo_url,
                    stderr = %String::from_utf8_lossy(&fetch.stderr),
                    "git fetch failed"
                );
            }
        } else {
            // Fresh clone
            let clone = Command::new("git")
                .args(["clone", "--", &self.repo_url, self.work_dir.to_str().unwrap_or(".")])
                .output()
                .await?;
            if !clone.status.success() {
                return Err(DeployError::Git(format!(
                    "git clone failed: {}",
                    String::from_utf8_lossy(&clone.stderr)
                )));
            }
        }

        // Checkout the target revision
        let checkout = Command::new("git")
            .args(["checkout", revision])
            .current_dir(&self.work_dir)
            .output()
            .await?;
        if !checkout.status.success() {
            return Err(DeployError::Git(format!(
                "git checkout {} failed: {}",
                revision,
                String::from_utf8_lossy(&checkout.stderr)
            )));
        }

        self.current_revision().await
    }

    /// Return the current HEAD commit SHA.
    pub async fn current_revision(&self) -> Result<String, DeployError> {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.work_dir)
            .output()
            .await?;
        if !out.status.success() {
            return Err(DeployError::Git("rev-parse HEAD failed".to_string()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Walk `path` (relative to repo root) and return all parsed manifests.
    pub async fn load_manifests(&self, path: &str, recurse: bool) -> Result<Vec<Manifest>, DeployError> {
        let target = self.work_dir.join(path);
        if !target.exists() {
            return Err(DeployError::Git(format!("path '{}' not found in repo", path)));
        }
        let mut manifests = Vec::new();
        collect_manifests(&target, recurse, &mut manifests)?;
        Ok(manifests)
    }
}

fn collect_manifests(
    dir: &Path,
    recurse: bool,
    out: &mut Vec<Manifest>,
) -> Result<(), DeployError> {
    let entries = std::fs::read_dir(dir).map_err(|e| DeployError::Git(e.to_string()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() && recurse {
            collect_manifests(&p, recurse, out)?;
        } else if p.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            let content = std::fs::read_to_string(&p).map_err(|e| DeployError::Git(e.to_string()))?;
            let parsed = parse_manifests(&content)?;
            out.extend(parsed);
        }
    }
    Ok(())
}

// ─── Manifest parsing ─────────────────────────────────────────────────────────

/// Parse a YAML string that may contain multiple `---` separated documents.
pub fn parse_manifests(yaml: &str) -> Result<Vec<Manifest>, DeployError> {
    let mut manifests = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let value: Value = Value::deserialize(doc)?;
        if value.is_null() {
            continue;
        }
        if let Some(m) = value_to_manifest(value)? {
            manifests.push(m);
        }
    }
    Ok(manifests)
}

fn value_to_manifest(raw: Value) -> Result<Option<Manifest>, DeployError> {
    let api_version = match raw["apiVersion"].as_str() {
        Some(v) => v.to_string(),
        None => return Ok(None),
    };
    let kind = match raw["kind"].as_str() {
        Some(k) => k.to_string(),
        None => return Ok(None),
    };
    let name = match raw["metadata"]["name"].as_str() {
        Some(n) => n.to_string(),
        None => return Ok(None),
    };
    let namespace = raw["metadata"]["namespace"].as_str().map(String::from);
    let sync_wave = extract_sync_wave(&raw);
    let hook_type = extract_hook_type(&raw);
    let hook_delete_policy =
        raw["metadata"]["annotations"][ANNOTATION_HOOK_DELETE_POLICY].as_str().map(String::from);

    Ok(Some(Manifest { api_version, kind, name, namespace, raw, sync_wave, hook_type, hook_delete_policy }))
}

/// Extract `argocd.argoproj.io/sync-wave` as an integer (default 0).
pub fn extract_sync_wave(raw: &Value) -> i32 {
    raw["metadata"]["annotations"][ANNOTATION_SYNC_WAVE]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Extract `argocd.argoproj.io/hook` as a `SyncHookType`.
pub fn extract_hook_type(raw: &Value) -> Option<SyncHookType> {
    let s = raw["metadata"]["annotations"][ANNOTATION_HOOK].as_str()?;
    match s {
        "PreSync" => Some(SyncHookType::PreSync),
        "Sync" => Some(SyncHookType::Sync),
        "PostSync" => Some(SyncHookType::PostSync),
        "SyncFail" => Some(SyncHookType::SyncFail),
        "Skip" => Some(SyncHookType::Skip),
        _ => None,
    }
}

// ─── Sync-wave ordering ───────────────────────────────────────────────────────

/// Group manifests into waves ordered by wave number.  Each inner Vec contains
/// all resources for one wave and must fully succeed before the next begins.
pub fn order_by_waves(mut manifests: Vec<Manifest>) -> Vec<Vec<Manifest>> {
    manifests.sort_by_key(|m| m.sync_wave);
    let mut waves: Vec<Vec<Manifest>> = Vec::new();
    let mut current_wave_num: Option<i32> = None;

    for m in manifests {
        if Some(m.sync_wave) != current_wave_num {
            current_wave_num = Some(m.sync_wave);
            waves.push(Vec::new());
        }
        waves.last_mut().unwrap().push(m);
    }
    waves
}

/// Separate PreSync / PostSync / SyncFail hooks from regular resources.
pub fn partition_by_phase(
    manifests: Vec<Manifest>,
) -> (Vec<Manifest>, Vec<Manifest>, Vec<Manifest>, Vec<Manifest>) {
    let mut pre_sync = Vec::new();
    let mut sync_phase = Vec::new();
    let mut post_sync = Vec::new();
    let mut sync_fail = Vec::new();

    for m in manifests {
        match &m.hook_type {
            Some(SyncHookType::PreSync) => pre_sync.push(m),
            Some(SyncHookType::PostSync) => post_sync.push(m),
            Some(SyncHookType::SyncFail) => sync_fail.push(m),
            Some(SyncHookType::Skip) => {} // discard
            _ => sync_phase.push(m),
        }
    }
    (pre_sync, sync_phase, post_sync, sync_fail)
}

// ─── Drift detection ──────────────────────────────────────────────────────────

/// Detect drift between desired manifests (from git) and live objects.
/// Returns all diffs; call `is_out_of_sync` to get a bool.
pub fn detect_drift(
    desired: &[Manifest],
    live: &[Value],
) -> (Vec<ResourceDiff>, SyncStatus) {
    let diffs = compute_diff(desired, live);
    let sync_status =
        if is_out_of_sync(&diffs) { SyncStatus::OutOfSync } else { SyncStatus::Synced };
    (diffs, sync_status)
}

// ─── Retry with backoff ───────────────────────────────────────────────────────

/// Parse a duration string like "5s", "2m", "1h" into a `Duration`.
pub fn parse_duration(s: &str) -> Duration {
    if let Some(secs) = s.strip_suffix('s') {
        Duration::from_secs(secs.parse().unwrap_or(5))
    } else if let Some(mins) = s.strip_suffix('m') {
        Duration::from_secs(mins.parse::<u64>().unwrap_or(1) * 60)
    } else if let Some(hours) = s.strip_suffix('h') {
        Duration::from_secs(hours.parse::<u64>().unwrap_or(1) * 3600)
    } else {
        Duration::from_secs(5)
    }
}

/// Compute the backoff sleep for retry `n` given the policy.
pub fn compute_backoff(base: Duration, factor: f64, max: Duration, n: u32) -> Duration {
    let secs = base.as_secs_f64() * factor.powi(n as i32);
    let result = Duration::from_secs_f64(secs);
    result.min(max)
}

// ─── Resource tracking helpers ────────────────────────────────────────────────

/// Return the label set that cave-deploy stamps on every managed resource.
pub fn managed_labels(app_name: &str) -> HashMap<String, String> {
    [
        (LABEL_MANAGED_BY.to_string(), CAVE_MANAGER.to_string()),
        (LABEL_APP_NAME.to_string(), app_name.to_string()),
    ]
    .into()
}

/// Return the annotation set used for annotation-based tracking.
pub fn managed_annotations(app_name: &str, instance: &str) -> HashMap<String, String> {
    [
        ("argocd.argoproj.io/app-name".to_string(), app_name.to_string()),
        ("argocd.argoproj.io/tracking-id".to_string(), format!("{app_name}:{instance}")),
    ]
    .into()
}

/// Inject tracking labels/annotations into a manifest's raw JSON.
pub fn inject_tracking(raw: &mut Value, app_name: &str) {
    if let Some(meta) = raw["metadata"].as_object_mut() {
        let labels = meta.entry("labels").or_insert_with(|| Value::Object(Default::default()));
        if let Some(lmap) = labels.as_object_mut() {
            lmap.insert(LABEL_MANAGED_BY.to_string(), Value::String(CAVE_MANAGER.to_string()));
            lmap.insert(LABEL_APP_NAME.to_string(), Value::String(app_name.to_string()));
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SIMPLE_DEPLOY: &str = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: myapp
  namespace: default
  annotations:
    argocd.argoproj.io/sync-wave: "2"
spec:
  replicas: 3
"#;

    const PRE_SYNC_JOB: &str = r#"
apiVersion: batch/v1
kind: Job
metadata:
  name: db-migrate
  namespace: default
  annotations:
    argocd.argoproj.io/hook: PreSync
    argocd.argoproj.io/sync-wave: "-5"
spec:
  template:
    spec:
      restartPolicy: Never
"#;

    const TWO_DOCS: &str = r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: config
  namespace: default
data:
  key: value
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: app
  namespace: default
  annotations:
    argocd.argoproj.io/sync-wave: "1"
spec:
  replicas: 1
"#;

    #[test]
    fn test_parse_single_manifest() {
        let manifests = parse_manifests(SIMPLE_DEPLOY).unwrap();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].kind, "Deployment");
        assert_eq!(manifests[0].name, "myapp");
        assert_eq!(manifests[0].namespace, Some("default".to_string()));
        assert_eq!(manifests[0].sync_wave, 2);
        assert!(manifests[0].hook_type.is_none());
    }

    #[test]
    fn test_parse_hook_manifest() {
        let manifests = parse_manifests(PRE_SYNC_JOB).unwrap();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].hook_type, Some(SyncHookType::PreSync));
        assert_eq!(manifests[0].sync_wave, -5);
    }

    #[test]
    fn test_parse_multi_document() {
        let manifests = parse_manifests(TWO_DOCS).unwrap();
        assert_eq!(manifests.len(), 2);
        let kinds: HashSet<&str> = manifests.iter().map(|m| m.kind.as_str()).collect();
        assert!(kinds.contains("ConfigMap"));
        assert!(kinds.contains("Deployment"));
    }

    #[test]
    fn test_order_by_waves() {
        let yaml = format!("{PRE_SYNC_JOB}\n---\n{SIMPLE_DEPLOY}");
        let manifests = parse_manifests(&yaml).unwrap();
        let waves = order_by_waves(manifests);
        assert_eq!(waves.len(), 2);
        // First wave is the pre-sync hook at wave -5
        assert_eq!(waves[0][0].sync_wave, -5);
        // Second wave is the deployment at wave 2
        assert_eq!(waves[1][0].sync_wave, 2);
    }

    #[test]
    fn test_detect_drift_synced() {
        let desired = parse_manifests(SIMPLE_DEPLOY).unwrap();
        // Live has exactly the same content (minus server fields which normalize strips)
        let live = vec![json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "myapp",
                "namespace": "default",
                "resourceVersion": "123",
                "annotations": { "argocd.argoproj.io/sync-wave": "2" }
            },
            "spec": { "replicas": 3 }
        })];
        let (_diffs, status) = detect_drift(&desired, &live);
        assert_eq!(status, SyncStatus::Synced);
    }

    #[test]
    fn test_detect_drift_out_of_sync() {
        let desired = parse_manifests(SIMPLE_DEPLOY).unwrap();
        let live: Vec<Value> = vec![]; // nothing in cluster
        let (_diffs, status) = detect_drift(&desired, &live);
        assert_eq!(status, SyncStatus::OutOfSync);
    }

    #[test]
    fn test_partition_by_phase() {
        let yaml = format!("{PRE_SYNC_JOB}\n---\n{SIMPLE_DEPLOY}");
        let manifests = parse_manifests(&yaml).unwrap();
        let (pre, sync, post, fail) = partition_by_phase(manifests);
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0].name, "db-migrate");
        assert_eq!(sync.len(), 1);
        assert_eq!(sync[0].name, "myapp");
        assert!(post.is_empty());
        assert!(fail.is_empty());
    }

    #[test]
    fn test_compute_backoff() {
        let base = Duration::from_secs(5);
        let max = Duration::from_secs(300);
        // retry 0: 5 * 2^0 = 5
        assert_eq!(compute_backoff(base, 2.0, max, 0), Duration::from_secs(5));
        // retry 1: 5 * 2^1 = 10
        assert_eq!(compute_backoff(base, 2.0, max, 1), Duration::from_secs(10));
        // retry 10 would exceed max → capped
        let capped = compute_backoff(base, 2.0, max, 10);
        assert_eq!(capped, max);
    }

    #[test]
    fn test_inject_tracking_labels() {
        let mut raw = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {"name": "app"}
        });
        inject_tracking(&mut raw, "my-app");
        assert_eq!(raw["metadata"]["labels"]["app.kubernetes.io/managed-by"], "cave-deploy");
        assert_eq!(raw["metadata"]["labels"]["argocd.argoproj.io/app-name"], "my-app");
    }
}

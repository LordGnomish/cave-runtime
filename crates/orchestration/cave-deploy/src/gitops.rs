// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sync engine bridge — drift detection, manifest rendering, auto-sync trigger.
//!
//! In MVP scope these functions operate against the in-memory store and do not
//! exec a real cluster client. The shape mirrors ArgoCD's reposerver +
//! application-controller boundary so a Phase 2 binding can drop in.

use crate::cluster::TRACKING_LABEL;
use crate::error::DeployError;
use crate::models::{
    Application, AutomatedSyncPolicy, HealthStatus, Manifest, SyncStatus,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use std::collections::HashMap;

/// Trigger a sync operation against the in-memory store.
///
/// Mirrors `controller/sync.go::Sync`. Returns the revision that was applied.
/// `force=true` skips the in-sync short-circuit.
pub fn sync_application(
    app: &mut Application,
    revision: Option<String>,
    force: bool,
) -> Result<String, DeployError> {
    let rev = revision.unwrap_or_else(|| {
        app.spec
            .source
            .target_revision
            .clone()
            .unwrap_or_else(|| "HEAD".to_string())
    });

    if !force {
        if let Some(status) = &app.status {
            if status.sync.status == SyncStatus::Synced && status.sync.revision == rev {
                return Ok(rev);
            }
        }
    }

    app.updated_at = Utc::now();
    Ok(rev)
}

/// Decide whether the application is in drift relative to its desired revision.
///
/// Returns `true` when the live state diverges from the spec — either the
/// stored sync status is OutOfSync, the last reconcile is older than the
/// requeue window (`max_age_minutes`), or no status has been observed yet.
pub fn detect_drift(app: &Application, max_age_minutes: i64) -> bool {
    let Some(status) = &app.status else {
        return true;
    };
    if status.sync.status == SyncStatus::OutOfSync {
        return true;
    }
    if let Some(reconciled) = status.reconciled_at {
        let age = Utc::now() - reconciled;
        return age > Duration::minutes(max_age_minutes);
    }
    true
}

/// Trigger an auto-sync if the policy permits it.
///
/// Mirrors `controller/sync.go::shouldAutoSync` + sync issuance.
/// Returns the revision that was applied, or `None` when no action was taken.
pub fn auto_sync(app: &mut Application) -> Option<String> {
    let policy = app.spec.sync_policy.as_ref()?;
    let automated: &AutomatedSyncPolicy = policy.automated.as_ref()?;

    let (current_sync, current_health) = match &app.status {
        Some(s) => (s.sync.status.clone(), s.health.status.clone()),
        None => (SyncStatus::Unknown, HealthStatus::Unknown),
    };

    let should = match current_sync {
        SyncStatus::OutOfSync => true,
        SyncStatus::Synced => automated.self_heal && current_health == HealthStatus::Degraded,
        SyncStatus::Unknown => false,
    };
    if !should {
        return None;
    }
    sync_application(app, None, false).ok()
}

/// Render Kubernetes manifests from an application source.
///
/// Returns parsed manifests, ready for diff/apply. In MVP scope this is a
/// shape-only renderer — Helm rendering goes through `helm template`, Kustomize
/// through `kustomize build`, and Directory/Git through raw YAML/JSON parsing.
pub fn render_manifests(app: &Application) -> Result<Vec<Manifest>, DeployError> {
    let src = &app.spec.source;
    let manifests = if src.helm.is_some() {
        render_helm(app)?
    } else if src.kustomize.is_some() {
        render_kustomize(app)?
    } else if let Some(dir) = &src.directory {
        render_directory(app, dir.recurse)?
    } else {
        render_directory(app, false)?
    };
    Ok(manifests
        .into_iter()
        .map(|m| inject_tracking_labels(m, &app.name))
        .collect())
}

fn render_helm(app: &Application) -> Result<Vec<Manifest>, DeployError> {
    let chart = app
        .spec
        .source
        .helm
        .as_ref()
        .and_then(|h| h.chart.clone())
        .unwrap_or_else(|| app.name.clone());
    let dep = manifest_template("apps/v1", "Deployment", &app.name, &app.spec.destination.namespace);
    let svc = manifest_template("v1", "Service", &chart, &app.spec.destination.namespace);
    Ok(vec![dep, svc])
}

fn render_kustomize(app: &Application) -> Result<Vec<Manifest>, DeployError> {
    let dep = manifest_template(
        "apps/v1",
        "Deployment",
        &app.name,
        &app.spec.destination.namespace,
    );
    Ok(vec![dep])
}

fn render_directory(app: &Application, _recurse: bool) -> Result<Vec<Manifest>, DeployError> {
    let dep = manifest_template(
        "apps/v1",
        "Deployment",
        &app.name,
        &app.spec.destination.namespace,
    );
    Ok(vec![dep])
}

fn manifest_template(api_version: &str, kind: &str, name: &str, namespace: &str) -> Manifest {
    let raw = serde_json::json!({
        "apiVersion": api_version,
        "kind": kind,
        "metadata": { "name": name, "namespace": namespace },
        "spec": {}
    });
    Manifest {
        api_version: api_version.to_string(),
        kind: kind.to_string(),
        name: name.to_string(),
        namespace: Some(namespace.to_string()),
        raw,
    }
}

fn inject_tracking_labels(mut m: Manifest, app_name: &str) -> Manifest {
    let labels = m.raw["metadata"]["labels"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut labels: HashMap<String, serde_json::Value> = labels.into_iter().collect();
    labels.insert(
        TRACKING_LABEL.to_string(),
        serde_json::Value::String(app_name.to_string()),
    );
    let labels_value: serde_json::Map<_, _> = labels.into_iter().collect();
    m.raw["metadata"]["labels"] = serde_json::Value::Object(labels_value);
    m
}

/// Parse a multi-document YAML stream into manifests.
pub fn parse_yaml_documents(yaml: &str) -> Result<Vec<Manifest>, DeployError> {
    let mut out = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let value: serde_yaml::Value = serde_yaml::Value::deserialize(doc)?;
        if value.is_null() {
            continue;
        }
        let json: serde_json::Value =
            serde_json::to_value(value).map_err(DeployError::from)?;
        if let Some(m) = Manifest::from_value(json) {
            out.push(m);
        }
    }
    Ok(out)
}

/// Parse a JSON document or JSON array into manifests.
pub fn parse_json_documents(json: &str) -> Result<Vec<Manifest>, DeployError> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let mut out = Vec::new();
    match value {
        serde_json::Value::Array(items) => {
            for v in items {
                if let Some(m) = Manifest::from_value(v) {
                    out.push(m);
                }
            }
        }
        v => {
            if let Some(m) = Manifest::from_value(v) {
                out.push(m);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn app(source: ApplicationSource) -> Application {
        Application {
            id: uuid::Uuid::new_v4(),
            name: "demo".into(),
            namespace: "argocd".into(),
            spec: ApplicationSpec {
                source,
                sources: vec![],
                destination: Destination {
                    server: "https://k.example".into(),
                    name: None,
                    namespace: "production".into(),
                },
                project: "default".into(),
                sync_policy: None,
                ignored_differences: None,
                info: None,
                revision_history_limit: None,
            },
            status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            labels: Default::default(),
            annotations: Default::default(),
            tracking: ResourceTracking::default(),
        }
    }

    #[test]
    fn render_helm_yields_dep_and_svc() {
        let mut a = app(ApplicationSource {
            repo_url: "https://charts.example".into(),
            target_revision: Some("1.0".into()),
            path: None,
            helm: Some(HelmSource {
                value_files: vec![],
                values: String::new(),
                parameters: vec![],
                file_parameters: vec![],
                release_name: Some("demo".into()),
                chart: Some("my-chart".into()),
                skip_crds: false,
                pass_credentials: false,
            }),
            kustomize: None,
            directory: None,
        });
        a.spec.destination.namespace = "prod".into();
        let out = render_manifests(&a).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|m| m.kind == "Service"));
        let dep = out.iter().find(|m| m.kind == "Deployment").unwrap();
        let label = dep.raw["metadata"]["labels"][TRACKING_LABEL].as_str();
        assert_eq!(label, Some("demo"));
    }

    #[test]
    fn render_kustomize_yields_dep() {
        let a = app(ApplicationSource {
            repo_url: "https://github.com/example/k".into(),
            target_revision: Some("main".into()),
            path: Some("overlays/prod".into()),
            helm: None,
            kustomize: Some(KustomizeSource {
                version: None,
                images: vec![],
                name_prefix: None,
                name_suffix: None,
                common_labels: Default::default(),
                common_annotations: Default::default(),
                patches: vec![],
            }),
            directory: None,
        });
        let out = render_manifests(&a).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn render_directory_yields_dep() {
        let a = app(ApplicationSource {
            repo_url: "https://github.com/example/y".into(),
            target_revision: Some("main".into()),
            path: Some("manifests/".into()),
            helm: None,
            kustomize: None,
            directory: Some(DirectorySource {
                recurse: true,
                include: None,
                exclude: None,
                jsonnet: None,
            }),
        });
        let out = render_manifests(&a).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn sync_application_returns_target_revision() {
        let mut a = app(ApplicationSource {
            repo_url: "r".into(),
            target_revision: Some("v1.0.0".into()),
            path: None,
            helm: None,
            kustomize: None,
            directory: None,
        });
        let rev = sync_application(&mut a, None, false).unwrap();
        assert_eq!(rev, "v1.0.0");
    }

    #[test]
    fn sync_application_explicit_revision_wins() {
        let mut a = app(ApplicationSource {
            repo_url: "r".into(),
            target_revision: Some("main".into()),
            path: None,
            helm: None,
            kustomize: None,
            directory: None,
        });
        let rev = sync_application(&mut a, Some("abc123".into()), true).unwrap();
        assert_eq!(rev, "abc123");
    }

    #[test]
    fn detect_drift_missing_status_is_drift() {
        let a = app(ApplicationSource {
            repo_url: "r".into(),
            target_revision: Some("main".into()),
            path: None,
            helm: None,
            kustomize: None,
            directory: None,
        });
        assert!(detect_drift(&a, 5));
    }

    #[test]
    fn detect_drift_out_of_sync() {
        let mut a = app(ApplicationSource {
            repo_url: "r".into(),
            target_revision: Some("main".into()),
            path: None,
            helm: None,
            kustomize: None,
            directory: None,
        });
        a.status = Some(ApplicationStatus {
            health: HealthCondition {
                status: HealthStatus::Healthy,
                message: None,
            },
            sync: SyncCondition {
                status: SyncStatus::OutOfSync,
                revision: "abc".into(),
                revisions: vec![],
            },
            resources: vec![],
            history: vec![],
            conditions: vec![],
            observed_at: Some(Utc::now()),
            reconciled_at: Some(Utc::now()),
        });
        assert!(detect_drift(&a, 60));
    }

    #[test]
    fn auto_sync_no_policy_returns_none() {
        let mut a = app(ApplicationSource {
            repo_url: "r".into(),
            target_revision: Some("main".into()),
            path: None,
            helm: None,
            kustomize: None,
            directory: None,
        });
        assert!(auto_sync(&mut a).is_none());
    }

    #[test]
    fn auto_sync_self_heal_on_degraded() {
        let mut a = app(ApplicationSource {
            repo_url: "r".into(),
            target_revision: Some("main".into()),
            path: None,
            helm: None,
            kustomize: None,
            directory: None,
        });
        a.spec.sync_policy = Some(SyncPolicy {
            automated: Some(AutomatedSyncPolicy {
                prune: false,
                self_heal: true,
                allow_empty: false,
            }),
            sync_options: vec![],
            retry: None,
            managed_namespace_metadata: None,
        });
        a.status = Some(ApplicationStatus {
            health: HealthCondition {
                status: HealthStatus::Degraded,
                message: None,
            },
            sync: SyncCondition {
                status: SyncStatus::Synced,
                revision: "abc".into(),
                revisions: vec![],
            },
            resources: vec![],
            history: vec![],
            conditions: vec![],
            observed_at: Some(Utc::now()),
            reconciled_at: Some(Utc::now()),
        });
        assert!(auto_sync(&mut a).is_some());
    }

    #[test]
    fn parse_yaml_documents_multi_doc() {
        let yaml = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\n  namespace: ns\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: b\n  namespace: ns\n";
        let out = parse_yaml_documents(yaml).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, "ConfigMap");
        assert_eq!(out[1].kind, "Service");
    }

    #[test]
    fn parse_json_documents_array() {
        let json = serde_json::json!([
            {"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"a","namespace":"n"}},
            {"apiVersion":"v1","kind":"Service","metadata":{"name":"b","namespace":"n"}},
        ]);
        let out = parse_json_documents(&json.to_string()).unwrap();
        assert_eq!(out.len(), 2);
    }
}

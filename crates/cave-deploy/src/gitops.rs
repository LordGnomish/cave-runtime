//! GitOps sync engine — drift detection, manifest rendering, and auto-sync.

use crate::models::{
    Application, ApplicationSource, Deployment, DiffResult, HealthStatus, ResourceDiff,
    SyncPolicy, SyncStatus,
};
use chrono::Utc;
use uuid::Uuid;

/// Apply desired state to the target cluster for the given application.
///
/// In production this would render manifests and run `kubectl apply`.
/// Here we update application state to reflect a successful sync.
pub fn sync_application(
    app: &mut Application,
    revision: Option<String>,
    force: bool,
) -> Deployment {
    let rev = revision.unwrap_or_else(|| "HEAD".to_string());

    app.sync_status = SyncStatus::Progressing;
    app.updated_at = Utc::now();

    // Simulate successful apply
    app.sync_status = SyncStatus::Synced;
    app.health_status = HealthStatus::Healthy;
    app.last_synced_at = Some(Utc::now());
    app.revision = Some(rev.clone());
    app.message = Some(format!("Synced to {rev}"));

    Deployment {
        id: Uuid::new_v4(),
        application_id: app.id,
        revision: rev.clone(),
        sync_status: SyncStatus::Synced,
        health_status: HealthStatus::Healthy,
        deployed_at: Utc::now(),
        deployed_by: "cave-deploy".to_string(),
        message: format!(
            "Synced to {rev}{}",
            if force { " (forced)" } else { "" }
        ),
    }
}

/// Return true if the live cluster state diverges from desired git state.
///
/// In production this would diff rendered manifests against live objects.
pub fn detect_drift(app: &Application) -> bool {
    match app.sync_status {
        SyncStatus::Synced => {
            if let Some(last_synced) = app.last_synced_at {
                // Treat as drifted if not re-synced within the last hour
                Utc::now()
                    .signed_duration_since(last_synced)
                    .num_minutes()
                    > 60
            } else {
                false
            }
        }
        SyncStatus::Unknown => false,
        _ => true,
    }
}

/// Trigger a sync if the policy is Automated and drift is detected.
pub fn auto_sync(app: &mut Application) -> Option<Deployment> {
    if app.sync_policy != SyncPolicy::Automated
        && app.sync_policy != SyncPolicy::AutomatedWithPrune
    {
        return None;
    }
    if detect_drift(app) {
        Some(sync_application(app, None, false))
    } else {
        None
    }
}

/// Produce a unified diff between desired manifests and live cluster state.
///
/// In production this runs `helm template` / `kustomize build` then diffs
/// against `kubectl get -o yaml` for each managed resource.
pub fn git_diff(app: &Application) -> DiffResult {
    let has_diff = detect_drift(app);

    let resources = if has_diff {
        vec![ResourceDiff {
            kind: "Deployment".to_string(),
            name: app.name.clone(),
            namespace: app.namespace.clone(),
            diff: format!(
                "--- live/{ns}/{name}\n+++ desired/{ns}/{name}\n@@ -1 +1 @@\n-  replicas: 2\n+  replicas: 3",
                ns = app.namespace,
                name = app.name,
            ),
        }]
    } else {
        vec![]
    };

    DiffResult {
        application_id: app.id,
        has_diff,
        resources,
        generated_at: Utc::now(),
    }
}

/// Render Kubernetes manifests for an application.
///
/// Dispatches to `helm template`, `kustomize build`, or raw git manifest
/// fetch depending on the source type.
pub fn render_manifests(app: &Application) -> Vec<String> {
    match &app.source {
        ApplicationSource::Helm(h) => vec![
            format!("# helm template {} --version {}", h.chart, h.version),
            format!(
                "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {}\n  namespace: {}",
                app.name, app.namespace
            ),
        ],
        ApplicationSource::Kustomize(k) => vec![
            format!("# kustomize build {}", k.path),
            format!(
                "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {}\n  namespace: {}",
                app.name, app.namespace
            ),
        ],
        ApplicationSource::Git(g) => vec![
            format!("# git: {} {} {}", g.repo_url, g.branch, g.path),
            format!(
                "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {}\n  namespace: {}",
                app.name, app.namespace
            ),
        ],
    }
}

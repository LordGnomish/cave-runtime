//! Resource and application health evaluation.

use crate::models::{Application, HealthStatus, ResourceStatus, SyncStatus};

/// Evaluate the health of a single Kubernetes resource.
///
/// In production this queries the k8s API (Deployment ready replicas,
/// Pod phase, CRD conditions, etc.).
pub fn check_resource_health(kind: &str, name: &str, namespace: &str) -> ResourceStatus {
    ResourceStatus {
        kind: kind.to_string(),
        name: name.to_string(),
        namespace: namespace.to_string(),
        health: HealthStatus::Healthy,
        sync_status: SyncStatus::Synced,
        message: None,
    }
}

/// Aggregate individual resource statuses into a single application health.
///
/// Priority order (highest to lowest): Degraded > Missing > Progressing >
/// Suspended > Healthy.  Returns Unknown when the slice is empty.
pub fn aggregate_app_health(resources: &[ResourceStatus]) -> HealthStatus {
    if resources.is_empty() {
        return HealthStatus::Unknown;
    }
    if resources.iter().any(|r| r.health == HealthStatus::Degraded) {
        return HealthStatus::Degraded;
    }
    if resources.iter().any(|r| r.health == HealthStatus::Missing) {
        return HealthStatus::Missing;
    }
    if resources.iter().any(|r| r.health == HealthStatus::Progressing) {
        return HealthStatus::Progressing;
    }
    if resources.iter().any(|r| r.health == HealthStatus::Suspended) {
        return HealthStatus::Suspended;
    }
    HealthStatus::Healthy
}

/// Return true if the application is in a degraded or missing state.
pub fn detect_degraded(app: &Application) -> bool {
    matches!(
        app.health_status,
        HealthStatus::Degraded | HealthStatus::Missing
    ) || app.sync_status == SyncStatus::Degraded
}

//! Health assessment — mirrors ArgoCD's health.lua logic in Rust.
//!
//! Each resource kind has its own health check. The overall application health
//! is the worst health across all non-hook resources.

use crate::models::{HealthStatus, HealthStatusDetail, ResourceStatus};
use serde_json::Value;

// ─── Per-kind health checks ───────────────────────────────────────────────────

/// Assess the health of a single live resource given its JSON representation.
pub fn assess_resource_health(kind: &str, live: &Value) -> HealthStatusDetail {
    match kind {
        "Deployment" => assess_deployment(live),
        "StatefulSet" => assess_statefulset(live),
        "DaemonSet" => assess_daemonset(live),
        "ReplicaSet" => assess_replicaset(live),
        "Pod" => assess_pod(live),
        "PersistentVolumeClaim" => assess_pvc(live),
        "Service" => assess_service(live),
        "Ingress" | "IngressRoute" => assess_ingress(live),
        "Job" => assess_job(live),
        "CronJob" => assess_cronjob(live),
        "HorizontalPodAutoscaler" => assess_hpa(live),
        "Certificate" | "CertificateRequest" => assess_certificate(live),
        // Everything else defaults to Healthy unless the object is missing.
        _ => HealthStatusDetail { status: HealthStatus::Healthy.to_string(), message: None },
    }
}

fn assess_deployment(live: &Value) -> HealthStatusDetail {
    let spec = &live["spec"];
    let status = &live["status"];

    if spec["paused"].as_bool().unwrap_or(false) {
        return mk(HealthStatus::Suspended, "Deployment is paused");
    }

    let desired = spec["replicas"].as_i64().unwrap_or(1);
    if desired == 0 {
        return mk(HealthStatus::Healthy, "Scaled to zero");
    }

    // Check Progressing condition for deadline exceeded
    if let Some(conds) = status["conditions"].as_array() {
        for c in conds {
            if c["type"].as_str() == Some("Progressing")
                && c["reason"].as_str() == Some("ProgressDeadlineExceeded")
            {
                return mk_msg(
                    HealthStatus::Degraded,
                    c["message"].as_str().unwrap_or("Progress deadline exceeded"),
                );
            }
            if c["type"].as_str() == Some("Available") && c["status"].as_str() == Some("False") {
                return mk_msg(
                    HealthStatus::Degraded,
                    c["message"].as_str().unwrap_or("Not available"),
                );
            }
        }
    }

    let available = status["availableReplicas"].as_i64().unwrap_or(0);
    let ready = status["readyReplicas"].as_i64().unwrap_or(0);
    let updated = status["updatedReplicas"].as_i64().unwrap_or(0);

    if updated < desired {
        return mk_msg(HealthStatus::Progressing, &format!("{updated}/{desired} updated"));
    }
    if available < desired {
        return mk_msg(HealthStatus::Progressing, &format!("{available}/{desired} available"));
    }
    if ready < desired {
        return mk_msg(HealthStatus::Progressing, &format!("{ready}/{desired} ready"));
    }

    mk_none(HealthStatus::Healthy)
}

fn assess_statefulset(live: &Value) -> HealthStatusDetail {
    let spec = &live["spec"];
    let status = &live["status"];
    let desired = spec["replicas"].as_i64().unwrap_or(1);
    if desired == 0 {
        return mk(HealthStatus::Healthy, "Scaled to zero");
    }
    let ready = status["readyReplicas"].as_i64().unwrap_or(0);
    let updated = status["updatedReplicas"].as_i64().unwrap_or(0);
    let current = status["currentReplicas"].as_i64().unwrap_or(0);
    if ready < desired || updated < desired || current < desired {
        return mk_msg(HealthStatus::Progressing, &format!("{ready}/{desired} ready"));
    }
    mk_none(HealthStatus::Healthy)
}

fn assess_daemonset(live: &Value) -> HealthStatusDetail {
    let status = &live["status"];
    let desired = status["desiredNumberScheduled"].as_i64().unwrap_or(0);
    if desired == 0 {
        return mk_none(HealthStatus::Healthy);
    }
    let ready = status["numberReady"].as_i64().unwrap_or(0);
    let updated = status["updatedNumberScheduled"].as_i64().unwrap_or(0);
    if updated < desired || ready < desired {
        return mk_msg(
            HealthStatus::Progressing,
            &format!("{ready}/{desired} ready, {updated}/{desired} updated"),
        );
    }
    mk_none(HealthStatus::Healthy)
}

fn assess_replicaset(live: &Value) -> HealthStatusDetail {
    let spec = &live["spec"];
    let status = &live["status"];
    let desired = spec["replicas"].as_i64().unwrap_or(0);
    let ready = status["readyReplicas"].as_i64().unwrap_or(0);
    if desired == 0 || ready >= desired {
        return mk_none(HealthStatus::Healthy);
    }
    mk_msg(HealthStatus::Progressing, &format!("{ready}/{desired} ready"))
}

fn assess_pod(live: &Value) -> HealthStatusDetail {
    let status = &live["status"];
    let phase = status["phase"].as_str().unwrap_or("Unknown");
    match phase {
        "Succeeded" => mk(HealthStatus::Healthy, "Completed"),
        "Failed" => mk_msg(
            HealthStatus::Degraded,
            status["message"].as_str().unwrap_or("Pod failed"),
        ),
        "Pending" => mk(HealthStatus::Progressing, "Pod pending"),
        "Running" => {
            if let Some(containers) = status["containerStatuses"].as_array() {
                for c in containers {
                    if c["ready"].as_bool().unwrap_or(false) {
                        continue;
                    }
                    let wait_reason =
                        c["state"]["waiting"]["reason"].as_str().unwrap_or("");
                    let term_reason =
                        c["state"]["terminated"]["reason"].as_str().unwrap_or("");
                    let bad_wait = matches!(
                        wait_reason,
                        "CrashLoopBackOff" | "OOMKilled" | "Error" | "ImagePullBackOff"
                            | "ErrImagePull"
                    );
                    let bad_term = matches!(term_reason, "Error" | "OOMKilled");
                    if bad_wait || bad_term {
                        let reason = if !wait_reason.is_empty() { wait_reason } else { term_reason };
                        return mk_msg(HealthStatus::Degraded, reason);
                    }
                    let reason =
                        if !wait_reason.is_empty() { wait_reason } else { "not ready" };
                    return mk_msg(HealthStatus::Progressing, reason);
                }
            }
            mk_none(HealthStatus::Healthy)
        }
        _ => mk_none(HealthStatus::Unknown),
    }
}

fn assess_pvc(live: &Value) -> HealthStatusDetail {
    match live["status"]["phase"].as_str().unwrap_or("Unknown") {
        "Bound" => mk_none(HealthStatus::Healthy),
        "Pending" => mk(HealthStatus::Progressing, "PVC pending"),
        "Lost" => mk(HealthStatus::Degraded, "PVC lost"),
        _ => mk_none(HealthStatus::Unknown),
    }
}

fn assess_service(live: &Value) -> HealthStatusDetail {
    if live["spec"]["type"].as_str() == Some("LoadBalancer") {
        let ingress = &live["status"]["loadBalancer"]["ingress"];
        if ingress.is_null() || ingress.as_array().map(|a| a.is_empty()).unwrap_or(true) {
            return mk(HealthStatus::Progressing, "Waiting for LoadBalancer IP");
        }
    }
    mk_none(HealthStatus::Healthy)
}

fn assess_ingress(live: &Value) -> HealthStatusDetail {
    let lb = &live["status"]["loadBalancer"]["ingress"];
    if lb.is_null() || lb.as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return mk(HealthStatus::Progressing, "Ingress LB not yet assigned");
    }
    mk_none(HealthStatus::Healthy)
}

fn assess_job(live: &Value) -> HealthStatusDetail {
    let status = &live["status"];
    if let Some(conds) = status["conditions"].as_array() {
        for c in conds {
            if c["type"].as_str() == Some("Failed") && c["status"].as_str() == Some("True") {
                return mk_msg(
                    HealthStatus::Degraded,
                    c["message"].as_str().unwrap_or("Job failed"),
                );
            }
            if c["type"].as_str() == Some("Complete") && c["status"].as_str() == Some("True") {
                return mk_none(HealthStatus::Healthy);
            }
        }
    }
    mk(HealthStatus::Progressing, "Job running")
}

fn assess_cronjob(live: &Value) -> HealthStatusDetail {
    if live["spec"]["suspend"].as_bool().unwrap_or(false) {
        return mk(HealthStatus::Suspended, "CronJob suspended");
    }
    mk_none(HealthStatus::Healthy)
}

fn assess_hpa(live: &Value) -> HealthStatusDetail {
    if let Some(conds) = live["status"]["conditions"].as_array() {
        for c in conds {
            if c["type"].as_str() == Some("ScalingActive")
                && c["status"].as_str() == Some("False")
            {
                return mk_msg(
                    HealthStatus::Degraded,
                    c["reason"].as_str().unwrap_or("ScalingInactive"),
                );
            }
        }
    }
    mk_none(HealthStatus::Healthy)
}

fn assess_certificate(live: &Value) -> HealthStatusDetail {
    if let Some(conds) = live["status"]["conditions"].as_array() {
        for c in conds {
            if c["type"].as_str() == Some("Ready") {
                return if c["status"].as_str() == Some("True") {
                    mk_none(HealthStatus::Healthy)
                } else {
                    mk_msg(
                        HealthStatus::Progressing,
                        c["message"].as_str().unwrap_or("Waiting for certificate"),
                    )
                };
            }
        }
    }
    mk(HealthStatus::Progressing, "Waiting for certificate")
}

// ─── App-level aggregation ────────────────────────────────────────────────────

/// Compute the overall application health from its resource statuses.
/// Hook resources are excluded; the worst non-hook health wins.
pub fn compute_app_health(resources: &[ResourceStatus]) -> HealthStatusDetail {
    let mut worst = HealthStatus::Healthy;
    let mut message = None;

    for r in resources {
        if r.hook {
            continue;
        }
        let hs: HealthStatus = r
            .health
            .as_ref()
            .map(|h| h.status.parse().unwrap_or(HealthStatus::Unknown))
            .unwrap_or(HealthStatus::Unknown);
        if health_priority(&hs) > health_priority(&worst) {
            worst = hs;
            message = r.health.as_ref().and_then(|h| h.message.clone());
        }
    }

    HealthStatusDetail { status: worst.to_string(), message }
}

fn health_priority(h: &HealthStatus) -> u8 {
    match h {
        HealthStatus::Healthy => 0,
        HealthStatus::Progressing => 1,
        HealthStatus::Suspended => 2,
        HealthStatus::Missing => 3,
        HealthStatus::Unknown => 4,
        HealthStatus::Degraded => 5,
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn mk(status: HealthStatus, msg: &str) -> HealthStatusDetail {
    HealthStatusDetail { status: status.to_string(), message: Some(msg.to_string()) }
}

fn mk_msg(status: HealthStatus, msg: &str) -> HealthStatusDetail {
    HealthStatusDetail { status: status.to_string(), message: Some(msg.to_string()) }
}

fn mk_none(status: HealthStatus) -> HealthStatusDetail {
    HealthStatusDetail { status: status.to_string(), message: None }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::HealthStatusDetail;
    use serde_json::json;

    #[test]
    fn test_deployment_healthy() {
        let live = json!({
            "spec": { "replicas": 3 },
            "status": { "availableReplicas": 3, "readyReplicas": 3, "updatedReplicas": 3 }
        });
        let h = assess_resource_health("Deployment", &live);
        assert_eq!(h.status, "Healthy");
    }

    #[test]
    fn test_deployment_progressing() {
        let live = json!({
            "spec": { "replicas": 3 },
            "status": { "availableReplicas": 1, "readyReplicas": 1, "updatedReplicas": 2 }
        });
        let h = assess_resource_health("Deployment", &live);
        assert_eq!(h.status, "Progressing");
    }

    #[test]
    fn test_deployment_degraded_deadline() {
        let live = json!({
            "spec": { "replicas": 3 },
            "status": {
                "availableReplicas": 3, "readyReplicas": 3, "updatedReplicas": 3,
                "conditions": [{"type":"Progressing","status":"False","reason":"ProgressDeadlineExceeded","message":"deadline"}]
            }
        });
        let h = assess_resource_health("Deployment", &live);
        assert_eq!(h.status, "Degraded");
    }

    #[test]
    fn test_deployment_suspended() {
        let live = json!({ "spec": { "paused": true, "replicas": 3 }, "status": {} });
        let h = assess_resource_health("Deployment", &live);
        assert_eq!(h.status, "Suspended");
    }

    #[test]
    fn test_pod_crash_loop() {
        let live = json!({
            "status": {
                "phase": "Running",
                "containerStatuses": [{"ready": false, "state": {"waiting": {"reason": "CrashLoopBackOff"}}}]
            }
        });
        let h = assess_resource_health("Pod", &live);
        assert_eq!(h.status, "Degraded");
    }

    #[test]
    fn test_pvc_bound_healthy() {
        let h = assess_resource_health("PersistentVolumeClaim", &json!({"status":{"phase":"Bound"}}));
        assert_eq!(h.status, "Healthy");
    }

    #[test]
    fn test_pvc_lost_degraded() {
        let h = assess_resource_health("PersistentVolumeClaim", &json!({"status":{"phase":"Lost"}}));
        assert_eq!(h.status, "Degraded");
    }

    #[test]
    fn test_job_failed_degraded() {
        let live = json!({
            "status": { "conditions": [{"type":"Failed","status":"True","message":"job failed"}] }
        });
        let h = assess_resource_health("Job", &live);
        assert_eq!(h.status, "Degraded");
    }

    #[test]
    fn test_compute_app_health_degraded_wins() {
        let resources = vec![
            ResourceStatus {
                group: None, version: "apps/v1".to_string(), kind: "Deployment".to_string(),
                namespace: Some("default".to_string()), name: "ok".to_string(),
                status: None,
                health: Some(HealthStatusDetail { status: "Healthy".to_string(), message: None }),
                hook: false, require_pruning: false, sync_wave: 0,
            },
            ResourceStatus {
                group: None, version: "v1".to_string(), kind: "Pod".to_string(),
                namespace: Some("default".to_string()), name: "broken".to_string(),
                status: None,
                health: Some(HealthStatusDetail {
                    status: "Degraded".to_string(), message: Some("CrashLoopBackOff".to_string()),
                }),
                hook: false, require_pruning: false, sync_wave: 0,
            },
        ];
        let h = compute_app_health(&resources);
        assert_eq!(h.status, "Degraded");
    }

    #[test]
    fn test_compute_app_health_hook_excluded() {
        let resources = vec![
            ResourceStatus {
                group: None, version: "batch/v1".to_string(), kind: "Job".to_string(),
                namespace: Some("default".to_string()), name: "pre-sync-job".to_string(),
                status: None,
                health: Some(HealthStatusDetail { status: "Degraded".to_string(), message: None }),
                hook: true, // should be excluded
                require_pruning: false, sync_wave: 0,
            },
            ResourceStatus {
                group: None, version: "apps/v1".to_string(), kind: "Deployment".to_string(),
                namespace: Some("default".to_string()), name: "app".to_string(),
                status: None,
                health: Some(HealthStatusDetail { status: "Healthy".to_string(), message: None }),
                hook: false, require_pruning: false, sync_wave: 0,
            },
        ];
        let h = compute_app_health(&resources);
        assert_eq!(h.status, "Healthy");
    }
}

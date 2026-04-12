//! Health assessment — custom health checks per resource type.

use crate::models::{HealthCondition, HealthStatus};
use std::collections::HashMap;

// ─── Health check function type ───────────────────────────────────────────────

pub type HealthCheckFn = fn(&serde_json::Value) -> HealthCondition;

/// Registry of per-resource-type health check functions.
pub struct HealthCheckRegistry {
    checks: HashMap<ResourceKey, HealthCheckFn>,
    /// Lua/CEL script-based custom checks.
    custom_checks: HashMap<ResourceKey, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceKey {
    pub group: String,
    pub version: String,
    pub kind: String,
}

impl ResourceKey {
    pub fn new(group: impl Into<String>, version: impl Into<String>, kind: impl Into<String>) -> Self {
        Self { group: group.into(), version: version.into(), kind: kind.into() }
    }

    pub fn core(version: impl Into<String>, kind: impl Into<String>) -> Self {
        Self::new("", version, kind)
    }
}

impl HealthCheckRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            checks: HashMap::new(),
            custom_checks: HashMap::new(),
        };
        reg.register_builtins();
        reg
    }

    pub fn register(&mut self, key: ResourceKey, check: HealthCheckFn) {
        self.checks.insert(key, check);
    }

    pub fn register_custom(&mut self, key: ResourceKey, script: impl Into<String>) {
        self.custom_checks.insert(key, script.into());
    }

    pub fn assess(&self, resource: &serde_json::Value) -> HealthCondition {
        let key = extract_resource_key(resource);
        if let Some(check) = self.checks.get(&key) {
            check(resource)
        } else {
            // Default: unknown health for unrecognized resource types.
            HealthCondition { status: HealthStatus::Unknown, message: None }
        }
    }

    fn register_builtins(&mut self) {
        self.register(ResourceKey::new("apps", "v1", "Deployment"), check_deployment);
        self.register(ResourceKey::new("apps", "v1", "StatefulSet"), check_statefulset);
        self.register(ResourceKey::new("apps", "v1", "DaemonSet"), check_daemonset);
        self.register(ResourceKey::new("apps", "v1", "ReplicaSet"), check_replicaset);
        self.register(ResourceKey::core("v1", "Pod"), check_pod);
        self.register(ResourceKey::core("v1", "PersistentVolumeClaim"), check_pvc);
        self.register(ResourceKey::core("v1", "Service"), check_service);
        self.register(ResourceKey::new("batch", "v1", "Job"), check_job);
        self.register(ResourceKey::new("batch", "v1", "CronJob"), check_cronjob);
        self.register(ResourceKey::new("networking.k8s.io", "v1", "Ingress"), check_ingress);
        self.register(ResourceKey::new("apiextensions.k8s.io", "v1", "CustomResourceDefinition"), check_crd);
        self.register(ResourceKey::new("cert-manager.io", "v1", "Certificate"), check_certificate);
        self.register(ResourceKey::new("argoproj.io", "v1alpha1", "Application"), check_argocd_app);
    }
}

fn extract_resource_key(resource: &serde_json::Value) -> ResourceKey {
    let api_version = resource.get("apiVersion").and_then(|v| v.as_str()).unwrap_or("");
    let kind = resource.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let (group, version) = if let Some(slash) = api_version.find('/') {
        (&api_version[..slash], &api_version[slash + 1..])
    } else {
        ("", api_version)
    };
    ResourceKey::new(group, version, kind)
}

// ─── Built-in health checks ──────────────────────────────────────────────────

pub fn check_deployment(resource: &serde_json::Value) -> HealthCondition {
    let status = &resource["status"];
    let available_replicas = status.get("availableReplicas").and_then(|v| v.as_i64()).unwrap_or(0);
    let desired_replicas = resource
        .get("spec").and_then(|s| s.get("replicas")).and_then(|v| v.as_i64())
        .unwrap_or(1);
    let updated_replicas = status.get("updatedReplicas").and_then(|v| v.as_i64()).unwrap_or(0);
    let ready_replicas = status.get("readyReplicas").and_then(|v| v.as_i64()).unwrap_or(0);

    // Check for progression condition
    if let Some(conditions) = status.get("conditions").and_then(|c| c.as_array()) {
        for cond in conditions {
            let condition_type = cond.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let condition_status = cond.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let reason = cond.get("reason").and_then(|v| v.as_str()).unwrap_or("");

            if condition_type == "Progressing" && condition_status == "False" {
                return HealthCondition {
                    status: HealthStatus::Degraded,
                    message: Some(format!("Deployment is not progressing: {}", reason)),
                };
            }

            if condition_type == "ReplicaFailure" && condition_status == "True" {
                return HealthCondition {
                    status: HealthStatus::Degraded,
                    message: Some("Replica failure".to_string()),
                };
            }
        }
    }

    if desired_replicas == 0 {
        return HealthCondition { status: HealthStatus::Suspended, message: Some("Deployment scaled to zero".to_string()) };
    }

    if ready_replicas >= desired_replicas && updated_replicas >= desired_replicas {
        HealthCondition { status: HealthStatus::Healthy, message: None }
    } else {
        HealthCondition {
            status: HealthStatus::Progressing,
            message: Some(format!("{}/{} replicas available", available_replicas, desired_replicas)),
        }
    }
}

pub fn check_statefulset(resource: &serde_json::Value) -> HealthCondition {
    let status = &resource["status"];
    let desired = resource.get("spec").and_then(|s| s.get("replicas")).and_then(|v| v.as_i64()).unwrap_or(1);
    let ready = status.get("readyReplicas").and_then(|v| v.as_i64()).unwrap_or(0);
    let current = status.get("currentReplicas").and_then(|v| v.as_i64()).unwrap_or(0);

    if desired == 0 {
        return HealthCondition { status: HealthStatus::Suspended, message: Some("StatefulSet scaled to zero".to_string()) };
    }

    if ready >= desired {
        HealthCondition { status: HealthStatus::Healthy, message: None }
    } else {
        HealthCondition {
            status: HealthStatus::Progressing,
            message: Some(format!("{}/{} pods ready", ready, desired)),
        }
    }
}

pub fn check_daemonset(resource: &serde_json::Value) -> HealthCondition {
    let status = &resource["status"];
    let desired = status.get("desiredNumberScheduled").and_then(|v| v.as_i64()).unwrap_or(0);
    let ready = status.get("numberReady").and_then(|v| v.as_i64()).unwrap_or(0);
    let updated = status.get("updatedNumberScheduled").and_then(|v| v.as_i64()).unwrap_or(0);

    if desired == 0 {
        return HealthCondition { status: HealthStatus::Healthy, message: Some("No nodes to schedule".to_string()) };
    }

    if ready >= desired && updated >= desired {
        HealthCondition { status: HealthStatus::Healthy, message: None }
    } else {
        HealthCondition {
            status: HealthStatus::Progressing,
            message: Some(format!("{}/{} nodes ready", ready, desired)),
        }
    }
}

pub fn check_replicaset(resource: &serde_json::Value) -> HealthCondition {
    let status = &resource["status"];
    let desired = resource.get("spec").and_then(|s| s.get("replicas")).and_then(|v| v.as_i64()).unwrap_or(1);
    let ready = status.get("readyReplicas").and_then(|v| v.as_i64()).unwrap_or(0);

    if desired == 0 {
        return HealthCondition { status: HealthStatus::Suspended, message: None };
    }
    if ready >= desired {
        HealthCondition { status: HealthStatus::Healthy, message: None }
    } else {
        HealthCondition { status: HealthStatus::Progressing, message: Some(format!("{}/{} ready", ready, desired)) }
    }
}

pub fn check_pod(resource: &serde_json::Value) -> HealthCondition {
    let status = &resource["status"];
    let phase = status.get("phase").and_then(|v| v.as_str()).unwrap_or("Unknown");
    match phase {
        "Running" => {
            // Check for CrashLoopBackOff in container statuses
            if let Some(containers) = status.get("containerStatuses").and_then(|c| c.as_array()) {
                for c in containers {
                    if let Some(waiting) = c.get("state").and_then(|s| s.get("waiting")) {
                        let reason = waiting.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                        if reason == "CrashLoopBackOff" || reason == "OOMKilled" || reason == "Error" {
                            return HealthCondition {
                                status: HealthStatus::Degraded,
                                message: Some(format!("Container in {} state", reason)),
                            };
                        }
                    }
                }
            }
            HealthCondition { status: HealthStatus::Healthy, message: None }
        }
        "Succeeded" => HealthCondition { status: HealthStatus::Healthy, message: Some("Pod completed".to_string()) },
        "Failed" => HealthCondition { status: HealthStatus::Degraded, message: Some("Pod failed".to_string()) },
        "Pending" => HealthCondition { status: HealthStatus::Progressing, message: Some("Pod pending".to_string()) },
        _ => HealthCondition { status: HealthStatus::Unknown, message: None },
    }
}

pub fn check_pvc(resource: &serde_json::Value) -> HealthCondition {
    let phase = resource.get("status").and_then(|s| s.get("phase")).and_then(|v| v.as_str()).unwrap_or("Unknown");
    match phase {
        "Bound" => HealthCondition { status: HealthStatus::Healthy, message: None },
        "Pending" => HealthCondition { status: HealthStatus::Progressing, message: Some("PVC pending".to_string()) },
        "Lost" => HealthCondition { status: HealthStatus::Degraded, message: Some("PVC volume lost".to_string()) },
        _ => HealthCondition { status: HealthStatus::Unknown, message: None },
    }
}

pub fn check_service(resource: &serde_json::Value) -> HealthCondition {
    let svc_type = resource.get("spec").and_then(|s| s.get("type")).and_then(|v| v.as_str()).unwrap_or("ClusterIP");
    if svc_type == "LoadBalancer" {
        let ingress = resource.get("status")
            .and_then(|s| s.get("loadBalancer"))
            .and_then(|lb| lb.get("ingress"))
            .and_then(|i| i.as_array());
        if ingress.map(|i| i.is_empty()).unwrap_or(true) {
            return HealthCondition {
                status: HealthStatus::Progressing,
                message: Some("Waiting for load balancer IP".to_string()),
            };
        }
    }
    HealthCondition { status: HealthStatus::Healthy, message: None }
}

pub fn check_job(resource: &serde_json::Value) -> HealthCondition {
    let status = &resource["status"];
    if status.get("completionTime").is_some() {
        HealthCondition { status: HealthStatus::Healthy, message: Some("Job completed".to_string()) }
    } else if let Some(conditions) = status.get("conditions").and_then(|c| c.as_array()) {
        for cond in conditions {
            if cond.get("type").and_then(|v| v.as_str()) == Some("Failed")
                && cond.get("status").and_then(|v| v.as_str()) == Some("True")
            {
                return HealthCondition { status: HealthStatus::Degraded, message: Some("Job failed".to_string()) };
            }
        }
        HealthCondition { status: HealthStatus::Progressing, message: Some("Job running".to_string()) }
    } else {
        HealthCondition { status: HealthStatus::Progressing, message: Some("Job running".to_string()) }
    }
}

pub fn check_cronjob(resource: &serde_json::Value) -> HealthCondition {
    let suspended = resource.get("spec").and_then(|s| s.get("suspend")).and_then(|v| v.as_bool()).unwrap_or(false);
    if suspended {
        HealthCondition { status: HealthStatus::Suspended, message: Some("CronJob suspended".to_string()) }
    } else {
        HealthCondition { status: HealthStatus::Healthy, message: None }
    }
}

pub fn check_ingress(resource: &serde_json::Value) -> HealthCondition {
    let lb_ingress = resource.get("status")
        .and_then(|s| s.get("loadBalancer"))
        .and_then(|lb| lb.get("ingress"))
        .and_then(|i| i.as_array());
    if lb_ingress.map(|i| i.is_empty()).unwrap_or(true) {
        HealthCondition { status: HealthStatus::Progressing, message: Some("Waiting for ingress address".to_string()) }
    } else {
        HealthCondition { status: HealthStatus::Healthy, message: None }
    }
}

pub fn check_crd(resource: &serde_json::Value) -> HealthCondition {
    if let Some(conditions) = resource.get("status").and_then(|s| s.get("conditions")).and_then(|c| c.as_array()) {
        for cond in conditions {
            if cond.get("type").and_then(|v| v.as_str()) == Some("Established")
                && cond.get("status").and_then(|v| v.as_str()) == Some("True")
            {
                return HealthCondition { status: HealthStatus::Healthy, message: None };
            }
        }
    }
    HealthCondition { status: HealthStatus::Progressing, message: Some("CRD not yet established".to_string()) }
}

pub fn check_certificate(resource: &serde_json::Value) -> HealthCondition {
    if let Some(conditions) = resource.get("status").and_then(|s| s.get("conditions")).and_then(|c| c.as_array()) {
        for cond in conditions {
            let ctype = cond.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let status = cond.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if ctype == "Ready" && status == "True" {
                return HealthCondition { status: HealthStatus::Healthy, message: None };
            }
            if ctype == "Ready" && status == "False" {
                let msg = cond.get("message").and_then(|v| v.as_str()).map(|s| s.to_string());
                return HealthCondition { status: HealthStatus::Degraded, message: msg };
            }
        }
    }
    HealthCondition { status: HealthStatus::Progressing, message: Some("Certificate provisioning".to_string()) }
}

pub fn check_argocd_app(resource: &serde_json::Value) -> HealthCondition {
    let health_status = resource.get("status")
        .and_then(|s| s.get("health"))
        .and_then(|h| h.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");

    let status = match health_status {
        "Healthy" => HealthStatus::Healthy,
        "Progressing" => HealthStatus::Progressing,
        "Degraded" => HealthStatus::Degraded,
        "Suspended" => HealthStatus::Suspended,
        "Missing" => HealthStatus::Missing,
        _ => HealthStatus::Unknown,
    };
    HealthCondition { status, message: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deployment_healthy() {
        let resource = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "spec": { "replicas": 3 },
            "status": {
                "availableReplicas": 3,
                "readyReplicas": 3,
                "updatedReplicas": 3
            }
        });
        let result = check_deployment(&resource);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn deployment_progressing() {
        let resource = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "spec": { "replicas": 3 },
            "status": { "availableReplicas": 1, "readyReplicas": 1, "updatedReplicas": 1 }
        });
        let result = check_deployment(&resource);
        assert_eq!(result.status, HealthStatus::Progressing);
    }

    #[test]
    fn deployment_suspended() {
        let resource = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "spec": { "replicas": 0 },
            "status": {}
        });
        let result = check_deployment(&resource);
        assert_eq!(result.status, HealthStatus::Suspended);
    }

    #[test]
    fn pod_crashloopbackoff() {
        let resource = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "status": {
                "phase": "Running",
                "containerStatuses": [{
                    "state": {
                        "waiting": { "reason": "CrashLoopBackOff" }
                    }
                }]
            }
        });
        let result = check_pod(&resource);
        assert_eq!(result.status, HealthStatus::Degraded);
    }

    #[test]
    fn pvc_bound() {
        let resource = json!({ "status": { "phase": "Bound" } });
        assert_eq!(check_pvc(&resource).status, HealthStatus::Healthy);
    }

    #[test]
    fn pvc_lost() {
        let resource = json!({ "status": { "phase": "Lost" } });
        assert_eq!(check_pvc(&resource).status, HealthStatus::Degraded);
    }

    #[test]
    fn job_completed() {
        let resource = json!({ "status": { "completionTime": "2024-01-01T00:00:00Z" } });
        assert_eq!(check_job(&resource).status, HealthStatus::Healthy);
    }

    #[test]
    fn job_failed() {
        let resource = json!({
            "status": {
                "conditions": [{ "type": "Failed", "status": "True" }]
            }
        });
        assert_eq!(check_job(&resource).status, HealthStatus::Degraded);
    }

    #[test]
    fn cronjob_suspended() {
        let resource = json!({ "spec": { "suspend": true }, "status": {} });
        assert_eq!(check_cronjob(&resource).status, HealthStatus::Suspended);
    }

    #[test]
    fn registry_assesses_deployment() {
        let reg = HealthCheckRegistry::new();
        let resource = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "spec": { "replicas": 1 },
            "status": { "availableReplicas": 1, "readyReplicas": 1, "updatedReplicas": 1 }
        });
        let result = reg.assess(&resource);
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn certificate_ready() {
        let resource = json!({
            "status": {
                "conditions": [{ "type": "Ready", "status": "True" }]
            }
        });
        assert_eq!(check_certificate(&resource).status, HealthStatus::Healthy);
    }
}

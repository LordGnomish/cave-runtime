//! Data models for the controller manager.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A Deployment that owns one or more ReplicaSets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub name: String,
    pub namespace: String,
    pub replicas: u32,
    pub selector: HashMap<String, String>,
    pub template: PodTemplate,
    pub status: DeploymentStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeploymentStatus {
    pub observed_replicas: u32,
    pub ready_replicas: u32,
    pub available_replicas: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaSet {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub owner_deployment: Option<String>,
    pub replicas: u32,
    pub selector: HashMap<String, String>,
    pub template: PodTemplate,
    pub status: ReplicaSetStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplicaSetStatus {
    pub ready_replicas: u32,
    pub available_replicas: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodTemplate {
    pub labels: HashMap<String, String>,
    pub containers: Vec<ContainerTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerTemplate {
    pub name: String,
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    pub namespace: String,
    pub completions: u32,
    pub parallelism: u32,
    pub template: PodTemplate,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStatus {
    pub active: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub completion_time: Option<DateTime<Utc>>,
}

/// A namespace lifecycle record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub name: String,
    pub phase: NamespacePhase,
    pub created_at: DateTime<Utc>,
    pub deletion_timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespacePhase {
    Active,
    Terminating,
}

/// EndpointSlice mirroring upstream Service → Pod IPs map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointSlice {
    pub name: String,
    pub namespace: String,
    pub service_name: String,
    pub addresses: Vec<String>,
    pub ready: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deployment_status_default_zeroed() {
        let s = DeploymentStatus::default();
        assert_eq!(s.observed_replicas, 0);
        assert_eq!(s.ready_replicas, 0);
        assert_eq!(s.available_replicas, 0);
    }

    #[test]
    fn test_replicaset_status_default_zeroed() {
        let s = ReplicaSetStatus::default();
        assert_eq!(s.ready_replicas, 0);
    }

    #[test]
    fn test_namespace_phase_serialization() {
        let s = serde_json::to_string(&NamespacePhase::Active).unwrap();
        assert_eq!(s, "\"Active\"");
    }

    #[test]
    fn test_pod_template_labels_serialize() {
        let mut labels = HashMap::new();
        labels.insert("app".to_string(), "web".to_string());
        let t = PodTemplate {
            labels,
            containers: vec![ContainerTemplate { name: "c".into(), image: "nginx".into() }],
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"app\":\"web\""));
    }
}

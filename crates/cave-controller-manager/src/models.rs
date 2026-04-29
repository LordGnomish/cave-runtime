//! Shared types for the controller manager and individual controllers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Controller registration / status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerInfo {
    pub name: String,
    pub state: ControllerState,
    pub last_sync: Option<DateTime<Utc>>,
    pub reconcile_count: u64,
    pub error_count: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControllerState {
    Stopped,
    Running,
    Failed,
}

impl ControllerInfo {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            state: ControllerState::Stopped,
            last_sync: None,
            reconcile_count: 0,
            error_count: 0,
            last_error: None,
        }
    }
}

/// A Deployment, observed by the deployment controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentObs {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub desired_replicas: u32,
    pub available_replicas: u32,
    pub selector: HashMap<String, String>,
}

/// A ReplicaSet, observed by the replica-set controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaSetObs {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub desired: u32,
    pub current: u32,
    pub owner_uid: Option<Uuid>,
}

/// A Node, observed by the node-lifecycle controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeObs {
    pub name: String,
    pub last_heartbeat: DateTime<Utc>,
    pub ready: bool,
}

/// A Namespace, observed by the namespace controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceObs {
    pub name: String,
    pub phase: String,
    pub finalizers: Vec<String>,
    pub deletion_timestamp: Option<DateTime<Utc>>,
}

/// A generic resource with an owner reference, used by the garbage collector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedResource {
    pub uid: Uuid,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub owner_uid: Option<Uuid>,
}

/// A Service, observed by the endpoints controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceObs {
    pub name: String,
    pub namespace: String,
    pub selector: HashMap<String, String>,
}

/// A backing Pod IP that the endpoints controller will project into Endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodEndpoint {
    pub pod_name: String,
    pub namespace: String,
    pub ip: String,
    pub labels: HashMap<String, String>,
    pub ready: bool,
}

/// Endpoints object produced by the endpoints controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointsObj {
    pub service_name: String,
    pub namespace: String,
    pub addresses: Vec<String>,
}

/// A ServiceAccount and any tokens projected for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAccountObs {
    pub name: String,
    pub namespace: String,
    pub default_token: Option<String>,
}

/// A Job + its pods, tracked by the job controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobObs {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub completions_required: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub completed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_controller_info_new_starts_stopped() {
        let info = ControllerInfo::new("deployment");
        assert_eq!(info.name, "deployment");
        assert_eq!(info.state, ControllerState::Stopped);
        assert_eq!(info.reconcile_count, 0);
    }

    #[test]
    fn test_controller_state_serialization() {
        let s = serde_json::to_string(&ControllerState::Running).unwrap();
        assert_eq!(s, "\"Running\"");
    }
}

//! Shared state for the controller manager.

use crate::models::*;
use dashmap::DashMap;
use uuid::Uuid;

/// Top-level state for cave-controller-manager.
pub struct ControllerManagerState {
    /// Deployments keyed by (namespace, name).
    pub deployments: DashMap<(String, String), Deployment>,
    /// ReplicaSets keyed by uid.
    pub replicasets: DashMap<Uuid, ReplicaSet>,
    /// Jobs keyed by (namespace, name).
    pub jobs: DashMap<(String, String), Job>,
    /// Namespaces keyed by name.
    pub namespaces: DashMap<String, Namespace>,
    /// EndpointSlices keyed by (namespace, name).
    pub endpoint_slices: DashMap<(String, String), EndpointSlice>,
}

impl ControllerManagerState {
    pub fn new() -> Self {
        Self {
            deployments: DashMap::new(),
            replicasets: DashMap::new(),
            jobs: DashMap::new(),
            namespaces: DashMap::new(),
            endpoint_slices: DashMap::new(),
        }
    }
}

impl Default for ControllerManagerState {
    fn default() -> Self {
        Self::new()
    }
}

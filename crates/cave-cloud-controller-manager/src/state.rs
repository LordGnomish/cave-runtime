//! State for the cloud controller manager.

use crate::models::*;
use crate::provider::{CloudProvider, MockProvider};
use dashmap::DashMap;
use std::sync::Arc;

pub struct CloudControllerState {
    pub nodes: DashMap<String, CloudNode>,
    pub load_balancers: DashMap<(String, String), LoadBalancer>,
    pub routes: DashMap<String, CloudRoute>,
    pub provider: Arc<dyn CloudProvider>,
}

impl CloudControllerState {
    pub fn new() -> Self {
        Self::with_provider(Arc::new(MockProvider::new()))
    }

    pub fn with_provider(provider: Arc<dyn CloudProvider>) -> Self {
        Self {
            nodes: DashMap::new(),
            load_balancers: DashMap::new(),
            routes: DashMap::new(),
            provider,
        }
    }
}

impl Default for CloudControllerState {
    fn default() -> Self { Self::new() }
}

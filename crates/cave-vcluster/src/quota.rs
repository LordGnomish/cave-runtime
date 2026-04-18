//! Per-namespace quota tracking.

use crate::models::{MAX_CLUSTERS_PER_NAMESPACE, QuotaStatus};
use dashmap::DashMap;

pub struct QuotaManager {
    limits: DashMap<String, u32>,
}

impl QuotaManager {
    pub fn new() -> Self {
        Self { limits: DashMap::new() }
    }

    pub fn get_max(&self, namespace: &str) -> u32 {
        self.limits.get(namespace).map(|r| *r).unwrap_or(MAX_CLUSTERS_PER_NAMESPACE)
    }

    pub fn set_max(&self, namespace: &str, max: u32) {
        self.limits.insert(namespace.to_owned(), max);
    }

    pub fn status(&self, namespace: &str, current: u32) -> QuotaStatus {
        let max = self.get_max(namespace);
        QuotaStatus {
            namespace: namespace.to_owned(),
            current_count: current,
            max_count: max,
            available: max.saturating_sub(current),
        }
    }
}

impl Default for QuotaManager {
    fn default() -> Self { Self::new() }
}

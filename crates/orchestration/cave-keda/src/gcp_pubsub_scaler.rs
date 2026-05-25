// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GCP Pub/Sub scaler — undelivered message count on a subscription.
//! upstream: kedacore/keda v2.x — pkg/scalers/gcp_pubsub_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct GcpPubSubScaler {
    pub tenant_id: String,
    pub project_id: String,
    pub subscription_name: String,
    pub subscription_size_target: i64,
    pub activation_threshold: i64,
    pub current_subscription_size: i64,
}

impl GcpPubSubScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            project_id: String::new(),
            subscription_name: String::new(),
            subscription_size_target: 100,
            activation_threshold: 0,
            current_subscription_size: 0,
        }
    }

    pub fn observe(&mut self, subscription_size: i64) {
        self.current_subscription_size = subscription_size.max(0);
    }
}

impl ScalerTrait for GcpPubSubScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_subscription_size as f64)
    }
    fn is_active(&self) -> bool {
        self.current_subscription_size > self.activation_threshold
    }
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_target_one_hundred() {
        let s = GcpPubSubScaler::new("t");
        assert_eq!(s.subscription_size_target, 100);
    }
}

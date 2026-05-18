// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus scaler — scales on a PromQL query result.
//! upstream: kedacore/keda v2.x — pkg/scalers/prometheus_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Default, Debug, Clone)]
pub struct PrometheusScaler {
    pub tenant_id: String,
    pub server_address: String,
    pub query: String,
    pub threshold: f64,
    pub activation_threshold: f64,
    pub current_value: f64,
}

impl PrometheusScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            server_address: String::new(),
            query: String::new(),
            threshold: 100.0,
            activation_threshold: 0.0,
            current_value: 0.0,
        }
    }

    pub fn observe(&mut self, value: f64) {
        self.current_value = if value.is_nan() { 0.0 } else { value };
    }
}

impl ScalerTrait for PrometheusScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_value)
    }

    fn is_active(&self) -> bool {
        self.current_value > self.activation_threshold
    }

    fn activation_threshold(&self) -> f64 {
        self.activation_threshold
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

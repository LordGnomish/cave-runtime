// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Datadog scaler — generic metric query result.
//! upstream: kedacore/keda v2.x — pkg/scalers/datadog_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DatadogScaler {
    pub tenant_id: String,
    pub query: String,
    pub query_value: f64,
    pub query_target: f64,
    pub activation_query_value: f64,
    pub site: String,
}

impl DatadogScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            query: String::new(),
            query_value: 0.0,
            query_target: 1.0,
            activation_query_value: 0.0,
            site: "datadoghq.com".to_string(),
        }
    }

    pub fn observe(&mut self, value: f64) {
        self.query_value = if value.is_nan() { 0.0 } else { value };
    }
}

impl ScalerTrait for DatadogScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.query_value)
    }
    fn is_active(&self) -> bool {
        self.query_value > self.activation_query_value
    }
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_site_is_us() {
        let s = DatadogScaler::new("t");
        assert_eq!(s.site, "datadoghq.com");
    }

    #[test]
    fn nan_observation_clamps_to_zero() {
        let mut s = DatadogScaler::new("t");
        s.observe(f64::NAN);
        assert_eq!(s.query_value, 0.0);
    }
}

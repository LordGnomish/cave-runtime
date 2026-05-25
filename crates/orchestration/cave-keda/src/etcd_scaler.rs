// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Etcd scaler — scale on a key's integer value (used for leader-election
//! style worker fan-out signals).
//! upstream: kedacore/keda v2.x — pkg/scalers/etcd_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct EtcdScaler {
    pub tenant_id: String,
    pub endpoints: Vec<String>,
    pub watch_key: String,
    pub target_value: i64,
    pub activation_threshold: i64,
    pub current_value: i64,
}

impl EtcdScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            endpoints: vec!["http://localhost:2379".to_string()],
            watch_key: String::new(),
            target_value: 10,
            activation_threshold: 0,
            current_value: 0,
        }
    }

    pub fn observe(&mut self, value: i64) {
        self.current_value = value.max(0);
    }
}

impl ScalerTrait for EtcdScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_value as f64)
    }
    fn is_active(&self) -> bool {
        self.current_value > self.activation_threshold
    }
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_endpoint_localhost_2379() {
        let s = EtcdScaler::new("t");
        assert_eq!(s.endpoints[0], "http://localhost:2379");
    }
}

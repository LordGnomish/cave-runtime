// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NATS JetStream scaler — pending-messages lag for a stream consumer.
//! upstream: kedacore/keda v2.x — pkg/scalers/nats_jetstream_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct NatsJetStreamScaler {
    pub tenant_id: String,
    pub stream: String,
    pub consumer: String,
    pub account: String,
    pub consumer_lag_target: i64,
    pub activation_lag_threshold: i64,
    pub current_lag: i64,
}

impl NatsJetStreamScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            stream: String::new(),
            consumer: String::new(),
            account: "$G".to_string(),
            consumer_lag_target: 10,
            activation_lag_threshold: 0,
            current_lag: 0,
        }
    }

    pub fn observe(&mut self, lag: i64) {
        self.current_lag = lag.max(0);
    }
}

impl ScalerTrait for NatsJetStreamScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_lag as f64)
    }
    fn is_active(&self) -> bool {
        self.current_lag > self.activation_lag_threshold
    }
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_account_is_global() {
        let s = NatsJetStreamScaler::new("t");
        assert_eq!(s.account, "$G");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AWS SQS scaler — scales on ApproximateNumberOfMessages.
//! upstream: kedacore/keda v2.x — pkg/scalers/aws_sqs_queue_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AwsSqsScaler {
    pub tenant_id: String,
    pub queue_url: String,
    pub queue_length_target: i64,
    pub activation_queue_length: i64,
    pub current_queue_length: i64,
    pub aws_region: String,
}

impl AwsSqsScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            queue_url: String::new(),
            queue_length_target: 5,
            activation_queue_length: 0,
            current_queue_length: 0,
            aws_region: "us-east-1".to_string(),
        }
    }

    pub fn observe(&mut self, queue_length: i64) {
        self.current_queue_length = queue_length.max(0);
    }
}

impl ScalerTrait for AwsSqsScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_queue_length as f64)
    }
    fn is_active(&self) -> bool {
        self.current_queue_length > self.activation_queue_length
    }
    fn activation_threshold(&self) -> f64 {
        self.activation_queue_length as f64
    }
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_target_is_five() {
        let s = AwsSqsScaler::new("t");
        assert_eq!(s.queue_length_target, 5);
    }

    #[test]
    fn observe_zero_inactive() {
        let s = AwsSqsScaler::new("t");
        assert!(!s.is_active());
    }
}

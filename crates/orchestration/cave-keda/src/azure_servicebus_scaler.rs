// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Azure Service Bus scaler — queue or topic subscription depth.
//! upstream: kedacore/keda v2.x — pkg/scalers/azure_servicebus_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureServiceBusEntity {
    Queue,
    Topic,
    Subscription,
}

#[derive(Debug, Clone)]
pub struct AzureServiceBusScaler {
    pub tenant_id: String,
    pub entity: AzureServiceBusEntity,
    pub namespace: String,
    pub entity_name: String,
    pub target_message_count: i64,
    pub activation_message_count: i64,
    pub current_message_count: i64,
}

impl AzureServiceBusScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            entity: AzureServiceBusEntity::Queue,
            namespace: String::new(),
            entity_name: String::new(),
            target_message_count: 5,
            activation_message_count: 0,
            current_message_count: 0,
        }
    }

    pub fn observe(&mut self, message_count: i64) {
        self.current_message_count = message_count.max(0);
    }
}

impl ScalerTrait for AzureServiceBusScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_message_count as f64)
    }
    fn is_active(&self) -> bool {
        self.current_message_count > self.activation_message_count
    }
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_entity_is_queue() {
        let s = AzureServiceBusScaler::new("t");
        assert_eq!(s.entity, AzureServiceBusEntity::Queue);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Azure Event Hub scaler — per-partition unprocessed event count.
//! upstream: kedacore/keda v2.x — pkg/scalers/azure_eventhub_scaler.go

use crate::scaler::ScalerTrait;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AzureEventHubScaler {
    pub tenant_id: String,
    pub event_hub_name: String,
    pub consumer_group: String,
    pub unprocessed_event_threshold: i64,
    pub activation_threshold: i64,
    /// Per-partition unprocessed event count.
    pub partitions: BTreeMap<u32, i64>,
}

impl AzureEventHubScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            event_hub_name: String::new(),
            consumer_group: "$Default".to_string(),
            unprocessed_event_threshold: 64,
            activation_threshold: 0,
            partitions: BTreeMap::new(),
        }
    }

    pub fn record_unprocessed(&mut self, partition: u32, count: i64) {
        self.partitions.insert(partition, count.max(0));
    }

    pub fn total_unprocessed(&self) -> i64 {
        self.partitions.values().sum()
    }
}

impl ScalerTrait for AzureEventHubScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.total_unprocessed() as f64)
    }
    fn is_active(&self) -> bool {
        self.total_unprocessed() > self.activation_threshold
    }
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_consumer_group_is_default_dollar() {
        let s = AzureEventHubScaler::new("t");
        assert_eq!(s.consumer_group, "$Default");
    }
}

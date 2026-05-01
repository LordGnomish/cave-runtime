//! Kafka scaler — scales on Kafka consumer-group lag.
//! upstream: kedacore/keda v2.x — pkg/scalers/kafka_scaler.go

use crate::scaler::ScalerTrait;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Default, Debug, Clone)]
pub struct KafkaScaler {
    pub tenant_id: String,
    pub bootstrap_servers: Vec<String>,
    pub consumer_group: String,
    pub topic: String,
    pub lag_threshold: Option<i64>,
    pub activation_lag_threshold: Option<i64>,
    /// Per-partition lag observed at last poll.
    pub partition_lag: HashMap<i32, i64>,
}

impl KafkaScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            bootstrap_servers: Vec::new(),
            consumer_group: String::new(),
            topic: String::new(),
            lag_threshold: Some(10),
            activation_lag_threshold: Some(0),
            partition_lag: HashMap::new(),
        }
    }

    pub fn record_lag(&mut self, partition: i32, lag: i64) {
        self.partition_lag.insert(partition, lag.max(0));
    }

    pub fn total_lag(&self) -> i64 {
        self.partition_lag.values().copied().sum()
    }

    /// Recommended replica count: a replica per `lag_threshold` worth of lag,
    /// capped at the partition count.
    pub fn recommended_replicas(&self) -> i32 {
        let threshold = self.lag_threshold.unwrap_or(10);
        if threshold <= 0 {
            return self.partition_lag.len() as i32;
        }
        let total = self.total_lag();
        let raw = (total as f64 / threshold as f64).ceil() as i32;
        raw.min(self.partition_lag.len() as i32)
    }
}

impl ScalerTrait for KafkaScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.total_lag() as f64)
    }

    fn is_active(&self) -> bool {
        self.total_lag() > self.activation_lag_threshold.unwrap_or(0)
    }

    fn activation_threshold(&self) -> f64 {
        self.activation_lag_threshold.unwrap_or(0) as f64
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

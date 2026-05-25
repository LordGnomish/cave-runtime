// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Redis scaler — scales on Redis list-length / stream-length.
//! upstream: kedacore/keda v2.x — pkg/scalers/redis_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedisDataType {
    List,
    Stream,
}

impl Default for RedisDataType {
    fn default() -> Self {
        RedisDataType::List
    }
}

#[derive(Default, Debug, Clone)]
pub struct RedisScaler {
    pub tenant_id: String,
    pub address: String,
    pub list_name: String,
    pub list_length_threshold: i64,
    pub activation_list_length: i64,
    pub data_type: RedisDataType,
    pub current_length: i64,
}

impl RedisScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            address: String::new(),
            list_name: String::new(),
            list_length_threshold: 5,
            activation_list_length: 0,
            data_type: RedisDataType::List,
            current_length: 0,
        }
    }

    pub fn observe(&mut self, length: i64) {
        self.current_length = length.max(0);
    }
}

impl ScalerTrait for RedisScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_length as f64)
    }

    fn is_active(&self) -> bool {
        self.current_length > self.activation_list_length
    }

    fn activation_threshold(&self) -> f64 {
        self.activation_list_length as f64
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

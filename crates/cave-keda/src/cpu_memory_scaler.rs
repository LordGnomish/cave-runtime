//! CPU + Memory scalers — wrap the K8s HPA's resource metrics.
//! upstream: kedacore/keda v2.x — pkg/scalers/cpu_memory_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceMetricType {
    Utilization,
    AverageValue,
}

impl Default for ResourceMetricType {
    fn default() -> Self {
        ResourceMetricType::Utilization
    }
}

#[derive(Default, Debug, Clone)]
pub struct CpuScaler {
    pub tenant_id: String,
    /// Target CPU utilization percentage (0..100) when using Utilization,
    /// or millicores per pod for AverageValue.
    pub target: i32,
    pub metric_type: ResourceMetricType,
    pub current: i32,
}

impl CpuScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            target: 80,
            metric_type: ResourceMetricType::Utilization,
            current: 0,
        }
    }

    pub fn observe(&mut self, value: i32) {
        self.current = value.max(0);
    }
}

impl ScalerTrait for CpuScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current as f64)
    }

    fn is_active(&self) -> bool {
        self.current > 0
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(15)
    }
}

#[derive(Default, Debug, Clone)]
pub struct MemoryScaler {
    pub tenant_id: String,
    pub target: i64, // bytes per pod or utilization%
    pub metric_type: ResourceMetricType,
    pub current: i64,
}

impl MemoryScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            target: 80,
            metric_type: ResourceMetricType::Utilization,
            current: 0,
        }
    }

    pub fn observe(&mut self, value: i64) {
        self.current = value.max(0);
    }
}

impl ScalerTrait for MemoryScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current as f64)
    }

    fn is_active(&self) -> bool {
        self.current > 0
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(15)
    }
}

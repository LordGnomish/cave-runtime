// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scaler trait + ScalingModifiers.
//! upstream: kedacore/keda v2.x — pkg/scalers/

use std::time::Duration;

/// A trait every scaler implementation honors.
/// Mirrors keda v2.x `pkg/scalers/scaler.go::Scaler` (subset).
pub trait ScalerTrait {
    /// Returns the current external metric value (queue depth, consumer lag,
    /// CPU%, etc.). For point-in-time scalers, this is the live value.
    fn metric_value(&self) -> Option<f64>;

    /// Returns true when the scaler thinks the workload should be active
    /// (i.e. min_replicas > 0). KEDA scales 0→1 the moment any scaler is
    /// active and falls back to 0 when ALL scalers report inactive.
    fn is_active(&self) -> bool;

    /// Activation threshold — below this value, the scaler reports inactive.
    fn activation_threshold(&self) -> f64 {
        0.0
    }

    /// Polling interval — how often the controller queries the scaler.
    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Common configuration carried by every scaler.
#[derive(Default, Debug, Clone)]
pub struct Scaler {
    pub tenant_id: String,
    pub polling_interval: Option<Duration>,
    pub cooldown_period: Option<Duration>,
    pub fallback_replicas: Option<i32>,
}

impl Scaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            polling_interval: Some(Duration::from_secs(30)),
            cooldown_period: Some(Duration::from_secs(300)),
            fallback_replicas: None,
        }
    }

    pub fn scale_to_zero(&mut self) {
        // No replica state at this layer; this is a no-op marker for parity
        // with KEDA's Scaler.Close() — the cooldown_period kicks in at the
        // ScaledObject layer.
    }

    pub fn fallback(&self) -> Option<i32> {
        self.fallback_replicas
    }
}

/// ScalingModifiers — formula-based scaling targets (KEDA v2.13+ AdvancedConfig).
#[derive(Default, Debug, Clone)]
pub struct ScalingModifiers {
    /// CEL/expression formula evaluated against metric values.
    pub formula: Option<String>,
    /// Target replica value.
    pub target: Option<i32>,
    /// Activation threshold replicas (0→1 trigger).
    pub activation_target: Option<i32>,
}

/// Compute desired replicas from a metric value and target. Mirrors HPA's
/// V2 algorithm: `desired = ceil(current_metric / target_value) * current_replicas`,
/// but for KEDA the simpler form is `desired = ceil(current / target_per_pod)`.
pub fn replicas_from_metric(metric: f64, target_per_pod: f64) -> i32 {
    if target_per_pod <= 0.0 {
        return 0;
    }
    (metric / target_per_pod).ceil().max(0.0) as i32
}

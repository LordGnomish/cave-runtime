// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Knative Pod Autoscaler (KPA) — concurrency-based scaling.
//! upstream: knative/serving v1.18.x — pkg/autoscaler/scaling/

use crate::meta::ObjectMeta;
use std::time::{Duration, Instant};

/// Autoscaler config — mirrors the `config-autoscaler` ConfigMap.
#[derive(Debug, Clone)]
pub struct AutoscalerConfig {
    /// Target concurrent requests per replica (KPA default = 100).
    pub target_concurrency: f64,
    /// Replicas allowed when under no load.
    pub min_scale: i32,
    /// Replica ceiling.
    pub max_scale: i32,
    /// Stable window (default 60s) — averaged metric horizon.
    pub stable_window: Duration,
    /// Panic window (default 6s) — quick reaction horizon.
    pub panic_window: Duration,
    /// Panic threshold — ratio above which panic mode activates.
    pub panic_threshold: f64,
    /// Time after last activity before scaling to zero.
    pub scale_to_zero_grace_period: Duration,
}

impl Default for AutoscalerConfig {
    fn default() -> Self {
        Self {
            target_concurrency: 100.0,
            min_scale: 0,
            max_scale: 1000,
            stable_window: Duration::from_secs(60),
            panic_window: Duration::from_secs(6),
            panic_threshold: 2.0,
            scale_to_zero_grace_period: Duration::from_secs(30),
        }
    }
}

/// Per-revision metric collector.
#[derive(Debug, Clone)]
pub struct AutoscalerMetric {
    /// Concurrency observation timestamp + value.
    samples: Vec<(Instant, f64)>,
    last_activity: Option<Instant>,
}

impl Default for AutoscalerMetric {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoscalerMetric {
    pub fn new() -> Self {
        Self { samples: Vec::new(), last_activity: None }
    }

    pub fn record(&mut self, concurrency: f64) {
        self.record_at(Instant::now(), concurrency);
    }

    pub fn record_at(&mut self, t: Instant, concurrency: f64) {
        self.samples.push((t, concurrency));
        if concurrency > 0.0 {
            self.last_activity = Some(t);
        }
    }

    /// Average concurrency over the trailing window ending at `now`.
    pub fn average(&self, now: Instant, window: Duration) -> f64 {
        let cutoff = now.checked_sub(window).unwrap_or(now);
        let in_window: Vec<f64> = self
            .samples
            .iter()
            .filter(|(t, _)| *t >= cutoff)
            .map(|(_, v)| *v)
            .collect();
        if in_window.is_empty() {
            return 0.0;
        }
        in_window.iter().sum::<f64>() / in_window.len() as f64
    }

    pub fn last_activity(&self) -> Option<Instant> {
        self.last_activity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoscalerMode {
    /// Smooth scaling using stable_window average.
    Stable,
    /// Aggressive scaling using panic_window average.
    Panic,
}

#[derive(Debug, Clone)]
pub struct ScaleDecision {
    pub desired_replicas: i32,
    pub mode: AutoscalerMode,
    pub stable_average: f64,
    pub panic_average: f64,
}

pub struct Autoscaler {
    pub metadata: ObjectMeta,
    pub config: AutoscalerConfig,
}

impl Autoscaler {
    pub fn new(tenant_id: &str, config: AutoscalerConfig) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            config,
        }
    }

    /// Compute the desired replica count. Implements upstream KPA logic:
    /// 1. compute stable average over stable_window
    /// 2. compute panic average over panic_window
    /// 3. enter panic if panic_avg / stable_avg > panic_threshold
    /// 4. desired = ceil(active_avg / target_concurrency), clamped to [min_scale, max_scale]
    /// 5. if no activity within grace_period AND min_scale=0 → 0
    pub fn decide(&self, metric: &AutoscalerMetric, now: Instant) -> ScaleDecision {
        let stable = metric.average(now, self.config.stable_window);
        let panic = metric.average(now, self.config.panic_window);

        let mode = if stable > 0.0 && panic / stable >= self.config.panic_threshold {
            AutoscalerMode::Panic
        } else {
            AutoscalerMode::Stable
        };

        // Scale-to-zero: no activity within grace period AND min_scale = 0.
        if self.config.min_scale == 0 {
            let inactive = match metric.last_activity {
                Some(t) => now.duration_since(t) >= self.config.scale_to_zero_grace_period,
                None => true,
            };
            if inactive {
                return ScaleDecision {
                    desired_replicas: 0,
                    mode,
                    stable_average: stable,
                    panic_average: panic,
                };
            }
        }

        let active = if mode == AutoscalerMode::Panic { panic } else { stable };
        let raw = (active / self.config.target_concurrency).ceil() as i32;
        let desired = raw.max(self.config.min_scale).min(self.config.max_scale).max(0);

        ScaleDecision {
            desired_replicas: desired,
            mode,
            stable_average: stable,
            panic_average: panic,
        }
    }
}

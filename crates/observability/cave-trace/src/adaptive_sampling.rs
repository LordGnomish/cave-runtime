//! Jaeger adaptive sampling — post-aggregation per-operation probability calculation.
//!
//! Ports jaeger `plugin/sampling/strategystore/adaptive`:
//!   - calculator/calculator.go (PercentageIncreaseCappedCalculator)
//!   - processor.go            (calculateProbability + throughput aggregation + clamping)
//!
//! The adaptive sampler observes per-service/per-operation throughput over a time
//! window and continuously nudges each operation's probabilistic sampling rate so
//! that the *sampled* QPS converges on a target (`target_samples_per_second`),
//! capping how fast the probability is allowed to grow.
//!
//! Upstream: jaegertracing/jaeger v1.52.0 (Apache-2.0).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Default cap on how much a probability may increase in a single calculation
/// window, expressed as a fraction of the old value (jaeger
/// `defaultPercentageIncreaseCap = 0.5`).
pub const DEFAULT_PERCENTAGE_INCREASE_CAP: f64 = 0.5;

/// Lower clamp on any computed sampling probability
/// (jaeger `minSamplingProbability = 1e-5`, i.e. 0.001%).
pub const MIN_SAMPLING_PROBABILITY: f64 = 1e-5;

/// Upper clamp on any computed sampling probability.
pub const MAX_SAMPLING_PROBABILITY: f64 = 1.0;

/// jaeger `defaultInitialSamplingProbability = 0.001`.
pub const DEFAULT_INITIAL_SAMPLING_PROBABILITY: f64 = 0.001;

/// jaeger `defaultMinSamplesPerSecond` — operations seen less often than this are
/// never *lowered* below their old probability (avoid starving rare endpoints).
pub const DEFAULT_MIN_SAMPLES_PER_SECOND: f64 = 1.0 / 60.0;

/// PercentageIncreaseCappedCalculator (calculator.go).
#[derive(Debug, Clone, Copy)]
pub struct PercentageIncreaseCappedCalculator {
    percentage_increase_cap: f64,
}

impl PercentageIncreaseCappedCalculator {
    /// A zero cap selects the jaeger default (0.5).
    pub fn new(percentage_increase_cap: f64) -> Self {
        let cap = if percentage_increase_cap == 0.0 {
            DEFAULT_PERCENTAGE_INCREASE_CAP
        } else {
            percentage_increase_cap
        };
        Self {
            percentage_increase_cap: cap,
        }
    }

    /// `Calculate(targetQPS, curQPS, oldProbability)`.
    ///
    /// `factor = targetQPS / curQPS`; the new probability is `old * factor`, but
    /// when growing (`factor > 1`) the per-window increase is capped so the result
    /// never exceeds `old * (1 + cap)`. Decreases are unrestricted. A zero current
    /// QPS leaves the probability unchanged (no observations to act on).
    pub fn calculate(&self, target_qps: f64, cur_qps: f64, old_probability: f64) -> f64 {
        if cur_qps == 0.0 {
            return old_probability;
        }
        let factor = target_qps / cur_qps;
        let mut new_probability = old_probability * factor;
        if factor > 1.0 {
            let percent_increase = (new_probability - old_probability) / old_probability;
            if percent_increase > self.percentage_increase_cap {
                new_probability = old_probability * (1.0 + self.percentage_increase_cap);
            }
        }
        new_probability
    }
}

impl Default for PercentageIncreaseCappedCalculator {
    fn default() -> Self {
        Self::new(DEFAULT_PERCENTAGE_INCREASE_CAP)
    }
}

/// An observed throughput record for a single service/operation over the
/// aggregation window (jaeger `model.Throughput`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Throughput {
    pub service: String,
    pub operation: String,
    /// Number of spans seen for this operation in the window.
    pub count: u64,
}

impl Throughput {
    pub fn new(service: impl Into<String>, operation: impl Into<String>, count: u64) -> Self {
        Self {
            service: service.into(),
            operation: operation.into(),
            count,
        }
    }
}

/// Adaptive sampling processor — owns the rolling probability table and recomputes
/// it from a fresh batch of throughput observations (processor.go).
#[derive(Debug, Clone)]
pub struct AdaptiveSamplingProcessor {
    target_samples_per_second: f64,
    initial_sampling_probability: f64,
    min_samples_per_second: f64,
    calculation_interval_secs: f64,
    calculator: PercentageIncreaseCappedCalculator,
    probabilities: HashMap<String, HashMap<String, f64>>,
}

impl AdaptiveSamplingProcessor {
    pub fn new(target_samples_per_second: f64, calculation_interval_secs: f64) -> Self {
        Self {
            target_samples_per_second,
            initial_sampling_probability: DEFAULT_INITIAL_SAMPLING_PROBABILITY,
            min_samples_per_second: DEFAULT_MIN_SAMPLES_PER_SECOND,
            calculation_interval_secs,
            calculator: PercentageIncreaseCappedCalculator::default(),
            probabilities: HashMap::new(),
        }
    }

    pub fn with_initial_probability(mut self, p: f64) -> Self {
        self.initial_sampling_probability = p;
        self
    }

    /// Convert a span count over the window into queries-per-second.
    pub fn qps(&self, count: u64) -> f64 {
        if self.calculation_interval_secs <= 0.0 {
            return 0.0;
        }
        count as f64 / self.calculation_interval_secs
    }

    /// Look up the current probability for an operation, falling back to the
    /// initial probability when unseen.
    pub fn probability_for(&self, service: &str, operation: &str) -> f64 {
        self.probabilities
            .get(service)
            .and_then(|ops| ops.get(operation))
            .copied()
            .unwrap_or(self.initial_sampling_probability)
    }

    /// Compute the next probability for one operation given its observed QPS.
    ///
    /// Clamped to `[MIN_SAMPLING_PROBABILITY, MAX_SAMPLING_PROBABILITY]`. Operations
    /// whose QPS is below `min_samples_per_second` are never lowered below their old
    /// probability (rare endpoints keep their sampling rate).
    pub fn calculate_probability(&self, service: &str, operation: &str, qps: f64) -> f64 {
        let old = self.probability_for(service, operation);
        let mut new_probability =
            self.calculator
                .calculate(self.target_samples_per_second, qps, old);
        // Rare operations (below the minimum sample rate) must not be lowered.
        if qps < self.min_samples_per_second {
            new_probability = new_probability.max(old);
        }
        new_probability
            .min(MAX_SAMPLING_PROBABILITY)
            .max(MIN_SAMPLING_PROBABILITY)
    }

    /// Recompute the whole probability table from a batch of throughput records and
    /// store it. Returns the new table (service -> operation -> probability).
    pub fn calculate_probabilities(
        &mut self,
        throughput: &[Throughput],
    ) -> HashMap<String, HashMap<String, f64>> {
        let mut table: HashMap<String, HashMap<String, f64>> = HashMap::new();
        for tp in throughput {
            let qps = self.qps(tp.count);
            let probability = self.calculate_probability(&tp.service, &tp.operation, qps);
            table
                .entry(tp.service.clone())
                .or_default()
                .insert(tp.operation.clone(), probability);
        }
        self.probabilities = table.clone();
        table
    }

    pub fn probabilities(&self) -> &HashMap<String, HashMap<String, f64>> {
        &self.probabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn calculate_decreases_unrestricted_when_over_target() {
        let c = PercentageIncreaseCappedCalculator::new(0.5);
        approx(c.calculate(10.0, 100.0, 0.5), 0.05);
    }

    #[test]
    fn calculate_caps_increase_to_one_plus_cap() {
        let c = PercentageIncreaseCappedCalculator::new(0.5);
        approx(c.calculate(10.0, 1.0, 0.1), 0.15);
    }

    #[test]
    fn calculate_small_increase_under_cap_is_applied_directly() {
        let c = PercentageIncreaseCappedCalculator::new(0.5);
        approx(c.calculate(12.0, 10.0, 0.1), 0.12);
    }

    #[test]
    fn calculate_zero_cap_uses_default() {
        let c = PercentageIncreaseCappedCalculator::new(0.0);
        approx(c.calculate(10.0, 1.0, 0.2), 0.30);
    }

    #[test]
    fn calculate_zero_qps_keeps_old() {
        let c = PercentageIncreaseCappedCalculator::new(0.5);
        approx(c.calculate(10.0, 0.0, 0.42), 0.42);
    }

    #[test]
    fn qps_divides_count_by_interval() {
        let p = AdaptiveSamplingProcessor::new(1.0, 60.0);
        approx(p.qps(120), 2.0);
    }

    #[test]
    fn probability_for_unseen_returns_initial() {
        let p = AdaptiveSamplingProcessor::new(1.0, 60.0).with_initial_probability(0.001);
        approx(p.probability_for("svc", "op"), 0.001);
    }

    #[test]
    fn calculate_probability_clamps_to_max() {
        let p = AdaptiveSamplingProcessor::new(1000.0, 1.0).with_initial_probability(0.9);
        let np = p.calculate_probability("svc", "op", 0.001);
        approx(np, MAX_SAMPLING_PROBABILITY);
    }

    #[test]
    fn calculate_probability_clamps_to_min() {
        let p = AdaptiveSamplingProcessor::new(0.0001, 1.0).with_initial_probability(0.5);
        let np = p.calculate_probability("svc", "op", 1_000_000.0);
        approx(np, MIN_SAMPLING_PROBABILITY);
    }

    #[test]
    fn rare_operation_is_not_lowered_below_old() {
        let p = AdaptiveSamplingProcessor::new(0.0001, 1.0).with_initial_probability(0.01);
        let np = p.calculate_probability("svc", "rare", 0.001);
        approx(np, 0.01);
    }

    #[test]
    fn calculate_probabilities_builds_and_stores_table() {
        let mut p = AdaptiveSamplingProcessor::new(2.0, 1.0).with_initial_probability(0.001);
        let tp = vec![
            Throughput::new("svcA", "op1", 100),
            Throughput::new("svcB", "op2", 4),
        ];
        let table = p.calculate_probabilities(&tp);
        assert!(table.contains_key("svcA"));
        assert!(table["svcA"].contains_key("op1"));
        let stored = p.probability_for("svcA", "op1");
        approx(stored, table["svcA"]["op1"]);
        approx(table["svcA"]["op1"], 2e-5);
    }
}

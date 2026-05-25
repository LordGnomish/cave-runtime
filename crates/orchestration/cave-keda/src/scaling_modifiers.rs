// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ScalingModifiers — formula-based replica recommendation across triggers.
//!
//! Upstream reference (KEDA v2.13+):
//!   pkg/scaling/scaledobject_modifiers.go
//!
//! Upstream uses a CEL expression engine over the trigger metric map.
//! The Cave port supports the three high-signal formulas seen in
//! practice — `sum(...)`, `max(...)`, `min(...)` — falling back to
//! `sum` for any unrecognised formula. Full CEL evaluation will reuse
//! the cave-apiserver CEL engine when it lands.

use std::collections::BTreeMap;

/// One trigger's metric output going into the ScalingModifiers
/// aggregation.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub name: String,
    pub metric: f64,
    pub is_active: bool,
}

impl Trigger {
    pub fn new(name: &str, metric: f64, is_active: bool) -> Self {
        Self {
            name: name.to_string(),
            metric,
            is_active,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScalingModifiersEvaluator {
    pub formula: String,
    pub target: f64,
    pub activation_target: Option<i32>,
    triggers: BTreeMap<String, Trigger>,
}

impl ScalingModifiersEvaluator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_trigger(&mut self, t: Trigger) {
        self.triggers.insert(t.name.clone(), t);
    }

    /// Evaluate the formula against the trigger map and return the
    /// recommended replica count.
    pub fn evaluate(&self) -> i32 {
        let values: Vec<f64> = self.triggers.values().map(|t| t.metric).collect();
        let metric = if let Some(args) = self.parse_formula("max") {
            args.iter()
                .filter_map(|n| self.triggers.get(n).map(|t| t.metric))
                .fold(f64::MIN, f64::max)
        } else if let Some(args) = self.parse_formula("min") {
            args.iter()
                .filter_map(|n| self.triggers.get(n).map(|t| t.metric))
                .fold(f64::MAX, f64::min)
        } else if let Some(args) = self.parse_formula("sum") {
            args.iter()
                .filter_map(|n| self.triggers.get(n).map(|t| t.metric))
                .sum()
        } else if values.is_empty() {
            0.0
        } else {
            // Unknown formula → sum of every trigger metric.
            values.iter().sum()
        };
        if self.target <= 0.0 {
            return 0;
        }
        (metric / self.target).ceil().max(0.0) as i32
    }

    /// Same metric calculation as [`evaluate`] but returns whether the
    /// scaler should be considered active per `activation_target`.
    pub fn is_active(&self) -> bool {
        match self.activation_target {
            None => self.triggers.values().any(|t| t.is_active),
            Some(threshold) => self.evaluate() > threshold,
        }
    }

    fn parse_formula(&self, name: &str) -> Option<Vec<String>> {
        let prefix = format!("{name}(");
        let trimmed = self.formula.trim();
        let rest = trimmed.strip_prefix(&prefix)?;
        let inner = rest.strip_suffix(')')?;
        Some(inner.split(',').map(|s| s.trim().to_string()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_evaluator_returns_zero() {
        let ev = ScalingModifiersEvaluator::new();
        assert_eq!(ev.evaluate(), 0);
    }

    #[test]
    fn target_zero_returns_zero_safely() {
        let mut ev = ScalingModifiersEvaluator::new();
        ev.formula = "max(a)".into();
        ev.target = 0.0;
        ev.add_trigger(Trigger::new("a", 5.0, true));
        assert_eq!(ev.evaluate(), 0);
    }
}

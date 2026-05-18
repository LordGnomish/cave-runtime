// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alerting rules: evaluate a condition and transition through pending→firing→resolved.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::error::Result;
use crate::model::{Labels, QueryResult};
use crate::promql::{parse, Engine};

/// Alert lifecycle state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AlertState {
    Inactive,
    Pending,
    Firing,
}

/// A currently active alert instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiringAlert {
    pub name: String,
    pub state: AlertState,
    pub labels: Labels,
    pub annotations: Labels,
    pub active_at_ms: i64,
    pub fired_at_ms: Option<i64>,
    pub value: f64,
}

/// An alerting rule definition.
#[derive(Debug, Clone)]
pub struct AlertRule {
    pub name: String,
    pub expr: String,
    /// Minimum duration the condition must be true before firing (milliseconds).
    pub for_ms: i64,
    pub labels: Labels,
    pub annotations: Labels,
    /// State per label fingerprint: (state, first_seen_ms)
    pub active: HashMap<u64, (AlertState, i64)>,
}

impl AlertRule {
    pub fn new(name: impl Into<String>, expr: impl Into<String>, for_ms: i64) -> Self {
        Self {
            name: name.into(),
            expr: expr.into(),
            for_ms,
            labels: Labels::new(),
            annotations: Labels::new(),
            active: HashMap::new(),
        }
    }

    pub fn with_labels(mut self, labels: Labels) -> Self { self.labels = labels; self }
    pub fn with_annotations(mut self, annotations: Labels) -> Self { self.annotations = annotations; self }

    /// Evaluate the alert rule at `ts_ms`. Returns all currently firing alerts.
    pub fn evaluate(&mut self, engine: &Engine, ts_ms: i64) -> Result<Vec<FiringAlert>> {
        let ast = parse(&self.expr)?;
        let result = engine.eval_instant(&ast, ts_ms)?;

        let currently_active: Vec<(Labels, f64)> = match result {
            QueryResult::InstantVector(iv) => iv,
            QueryResult::Scalar(v) if v != 0.0 && !v.is_nan() => vec![(Labels::new(), v)],
            _ => vec![],
        };

        // Build set of currently active fingerprints
        let active_fps: std::collections::HashSet<u64> = currently_active.iter()
            .map(|(l, _)| l.fingerprint())
            .collect();

        // Remove fingerprints no longer active
        self.active.retain(|fp, _| active_fps.contains(fp));

        let mut out = Vec::new();

        for (series_labels, value) in currently_active {
            let mut alert_labels = series_labels.clone();
            for (k, v) in self.labels.iter() { alert_labels.insert(k, v); }

            let fp = series_labels.fingerprint();

            let (state, first_seen_ms) = self.active.entry(fp).or_insert((AlertState::Pending, ts_ms));

            // Transition Pending → Firing after `for_ms`
            if *state == AlertState::Pending && (ts_ms - *first_seen_ms) >= self.for_ms {
                *state = AlertState::Firing;
            }

            let fired_at_ms = if *state == AlertState::Firing { Some(*first_seen_ms + self.for_ms) } else { None };

            out.push(FiringAlert {
                name: self.name.clone(),
                state: state.clone(),
                labels: alert_labels,
                annotations: self.annotations.clone(),
                active_at_ms: *first_seen_ms,
                fired_at_ms,
                value,
            });
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsdb::Tsdb;
    use crate::model::{LabelMatcher, Sample};
    use std::sync::Arc;

    #[test]
    fn test_alert_pending_to_firing() {
        let tsdb = Arc::new(Tsdb::default());
        // Seed: a metric that is always 1
        tsdb.append(
            Labels::from_pairs([("__name__", "error_rate")]),
            Sample::new(0, 1.0),
        );
        tsdb.append(
            Labels::from_pairs([("__name__", "error_rate")]),
            Sample::new(60_000, 1.0),
        );

        let engine = Engine::new(Arc::clone(&tsdb));
        let mut rule = AlertRule::new("HighErrorRate", "error_rate > 0", 60_000); // for: 1m

        // t=0: pending
        let alerts = rule.evaluate(&engine, 0).unwrap();
        assert_eq!(alerts[0].state, AlertState::Pending);

        // t=60s: still pending (not yet >= for_ms)
        let alerts = rule.evaluate(&engine, 59_999).unwrap();
        assert_eq!(alerts[0].state, AlertState::Pending);

        // t=61s: firing
        let alerts = rule.evaluate(&engine, 60_001).unwrap();
        assert_eq!(alerts[0].state, AlertState::Firing);
    }
}

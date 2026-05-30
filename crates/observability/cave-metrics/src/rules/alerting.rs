// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Alerting rules: evaluate a condition and transition through pending→firing→resolved.

use crate::error::Result;
use crate::model::{Labels, QueryResult};
use crate::promql::{Engine, parse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Duration a resolved alert is retained in the Firing state before it is
    /// removed (milliseconds). Prometheus `keep_firing_for` (PR #11827). 0 = off.
    pub keep_firing_for_ms: i64,
    pub labels: Labels,
    pub annotations: Labels,
    /// State per label fingerprint: (state, first_seen_ms)
    pub active: HashMap<u64, (AlertState, i64)>,
    /// For Firing series whose condition has resolved: the timestamp at which
    /// resolution was first observed. Cleared if the series fires again.
    pub keep_firing_since: HashMap<u64, i64>,
    /// Last observed (labels, value) per fingerprint, used to keep emitting an
    /// alert during its keep_firing_for grace window.
    pub last_seen: HashMap<u64, (Labels, f64)>,
}

impl AlertRule {
    pub fn new(name: impl Into<String>, expr: impl Into<String>, for_ms: i64) -> Self {
        Self {
            name: name.into(),
            expr: expr.into(),
            for_ms,
            keep_firing_for_ms: 0,
            labels: Labels::new(),
            annotations: Labels::new(),
            active: HashMap::new(),
            keep_firing_since: HashMap::new(),
            last_seen: HashMap::new(),
        }
    }

    pub fn with_labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }
    pub fn with_annotations(mut self, annotations: Labels) -> Self {
        self.annotations = annotations;
        self
    }
    /// Set the `keep_firing_for` grace duration (milliseconds).
    pub fn with_keep_firing_for(mut self, keep_firing_for_ms: i64) -> Self {
        self.keep_firing_for_ms = keep_firing_for_ms;
        self
    }

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
        let active_fps: std::collections::HashSet<u64> = currently_active
            .iter()
            .map(|(l, _)| l.fingerprint())
            .collect();

        // Resolve fingerprints that are no longer active. A Firing series with a
        // keep_firing_for window is retained (still emitted as Firing) until the
        // grace period elapses; everything else is dropped immediately.
        let resolved_fps: Vec<u64> = self
            .active
            .keys()
            .copied()
            .filter(|fp| !active_fps.contains(fp))
            .collect();
        for fp in resolved_fps {
            let is_firing = matches!(self.active.get(&fp), Some((AlertState::Firing, _)));
            if is_firing && self.keep_firing_for_ms > 0 {
                let since = *self.keep_firing_since.entry(fp).or_insert(ts_ms);
                if ts_ms - since >= self.keep_firing_for_ms {
                    self.active.remove(&fp);
                    self.keep_firing_since.remove(&fp);
                    self.last_seen.remove(&fp);
                }
                // else: retain — still firing within the grace window.
            } else {
                self.active.remove(&fp);
                self.keep_firing_since.remove(&fp);
                self.last_seen.remove(&fp);
            }
        }

        let mut out = Vec::new();

        for (series_labels, value) in currently_active {
            let fp = series_labels.fingerprint();
            // The condition is true again → clear any pending resolution timer.
            self.keep_firing_since.remove(&fp);
            self.last_seen.insert(fp, (series_labels.clone(), value));

            let mut alert_labels = series_labels.clone();
            for (k, v) in self.labels.iter() {
                alert_labels.insert(k, v);
            }

            let (state, first_seen_ms) = self
                .active
                .entry(fp)
                .or_insert((AlertState::Pending, ts_ms));

            // Transition Pending → Firing after `for_ms`
            if *state == AlertState::Pending && (ts_ms - *first_seen_ms) >= self.for_ms {
                *state = AlertState::Firing;
            }

            let fired_at_ms = if *state == AlertState::Firing {
                Some(*first_seen_ms + self.for_ms)
            } else {
                None
            };

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

        // Emit alerts still held open by their keep_firing_for grace window.
        for (fp, since) in &self.keep_firing_since {
            let Some((state, first_seen_ms)) = self.active.get(fp) else {
                continue;
            };
            if *state != AlertState::Firing {
                continue;
            }
            let (series_labels, value) = match self.last_seen.get(fp) {
                Some(v) => v,
                None => continue,
            };
            let mut alert_labels = series_labels.clone();
            for (k, v) in self.labels.iter() {
                alert_labels.insert(k, v);
            }
            let _ = since;
            out.push(FiringAlert {
                name: self.name.clone(),
                state: AlertState::Firing,
                labels: alert_labels,
                annotations: self.annotations.clone(),
                active_at_ms: *first_seen_ms,
                fired_at_ms: Some(*first_seen_ms + self.for_ms),
                value: *value,
            });
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LabelMatcher, Sample};
    use crate::tsdb::Tsdb;
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

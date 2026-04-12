//! Alerting rules.

#![allow(dead_code)]

use std::collections::HashMap;
use crate::error::{MetricsError, MetricsResult};
use crate::model::Labels;
use crate::promql::{Engine, EvalContext, QueryValue};
use crate::promql::parser::parse;
use crate::tsdb::Tsdb;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertState {
    Pending,
    Firing,
    Resolved,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub name: String,
    pub labels: Labels,
    pub annotations: Labels,
    pub state: AlertState,
    pub fired_at: i64,
    pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AlertingRule {
    pub name: String,
    pub expr: String,
    pub for_ms: u64,
    pub labels: Labels,
    pub annotations: Labels,
    pub interval_ms: u64,
}

impl AlertingRule {
    pub async fn evaluate(
        &self,
        engine: &Engine,
        tsdb: &Tsdb,
        now_ms: i64,
        pending: &mut HashMap<String, i64>,
    ) -> MetricsResult<Vec<Alert>> {
        let expr = parse(&self.expr)?;
        let ctx = EvalContext::instant(now_ms);
        let result = engine.eval_instant(&expr, &ctx, tsdb)?;
        let active_fps = match result {
            QueryValue::InstantVector(samples) => {
                // Filter out NaN values (non-firing)
                samples.into_iter()
                    .filter(|s| !s.value.is_nan())
                    .collect::<Vec<_>>()
            }
            _ => return Err(MetricsError::Eval("alerting rule must return instant vector".to_string())),
        };

        let mut alerts = Vec::new();

        for sample in &active_fps {
            // Merge rule labels with series labels
            let mut alert_labels = self.labels.0.clone();
            for (k, v) in &sample.labels.0 {
                if k != "__name__" {
                    alert_labels.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            alert_labels.insert("alertname".to_string(), self.name.clone());
            let labels = Labels(alert_labels);
            let key = format!("{}", labels.fingerprint());

            let fired_at = *pending.entry(key.clone()).or_insert(now_ms);
            let pending_duration_ms = now_ms - fired_at;

            if pending_duration_ms >= self.for_ms as i64 {
                alerts.push(Alert {
                    name: self.name.clone(),
                    labels,
                    annotations: self.annotations.clone(),
                    state: AlertState::Firing,
                    fired_at,
                    resolved_at: None,
                });
            } else {
                alerts.push(Alert {
                    name: self.name.clone(),
                    labels,
                    annotations: self.annotations.clone(),
                    state: AlertState::Pending,
                    fired_at,
                    resolved_at: None,
                });
            }
        }

        // Clean up pending entries for series that are no longer active
        let active_keys: std::collections::HashSet<String> = active_fps.iter().map(|s| {
            let mut alert_labels = self.labels.0.clone();
            for (k, v) in &s.labels.0 {
                if k != "__name__" {
                    alert_labels.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            alert_labels.insert("alertname".to_string(), self.name.clone());
            format!("{}", Labels(alert_labels).fingerprint())
        }).collect();

        pending.retain(|k, _| active_keys.contains(k));

        Ok(alerts)
    }
}

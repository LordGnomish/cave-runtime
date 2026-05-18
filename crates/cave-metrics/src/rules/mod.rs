// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alerting and recording rules evaluation engine.

pub mod alerting;
pub mod recording;

pub use alerting::{AlertRule, AlertState, FiringAlert};
pub use recording::RecordingRule;

use crate::error::Result;
use crate::model::Labels;
use crate::promql::Engine;
use crate::tsdb::Tsdb;
use std::sync::Arc;
use std::time::Duration;

/// A rule group contains a set of rules evaluated on a shared interval.
pub struct RuleGroup {
    pub name: String,
    pub interval: Duration,
    pub recording_rules: Vec<RecordingRule>,
    pub alert_rules: Vec<AlertRule>,
}

impl RuleGroup {
    /// Evaluate all rules in this group at the current time.
    pub fn evaluate(&mut self, engine: &Engine, tsdb: &Arc<Tsdb>, ts_ms: i64) -> Result<Vec<FiringAlert>> {
        // Recording rules first
        for rule in &self.recording_rules {
            rule.evaluate(engine, tsdb, ts_ms)?;
        }

        // Alert rules
        let mut alerts = Vec::new();
        for rule in &mut self.alert_rules {
            let fired = rule.evaluate(engine, ts_ms)?;
            alerts.extend(fired);
        }
        Ok(alerts)
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Recording rules: evaluate a PromQL expression and write back to TSDB.

use crate::error::Result;
use crate::model::{Labels, QueryResult, Sample};
use crate::promql::{parse, Engine};
use crate::tsdb::Tsdb;
use std::sync::Arc;

/// A recording rule: `record: <name>` with `expr: <promql>` and optional labels.
#[derive(Debug, Clone)]
pub struct RecordingRule {
    pub name: String,
    pub expr: String,
    pub labels: Labels,
}

impl RecordingRule {
    pub fn new(name: impl Into<String>, expr: impl Into<String>) -> Self {
        Self { name: name.into(), expr: expr.into(), labels: Labels::new() }
    }

    pub fn with_labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }

    /// Evaluate the expression and write the result back to the TSDB.
    pub fn evaluate(&self, engine: &Engine, tsdb: &Arc<Tsdb>, ts_ms: i64) -> Result<()> {
        let ast = parse(&self.expr)?;
        let result = engine.eval_instant(&ast, ts_ms)?;

        match result {
            QueryResult::InstantVector(iv) => {
                for (mut labels, value) in iv {
                    // Set the __name__ to the recording rule name and merge extra labels.
                    labels.insert("__name__", &self.name);
                    for (k, v) in self.labels.iter() {
                        labels.insert(k, v);
                    }
                    tsdb.append(labels, Sample::new(ts_ms, value));
                }
            }
            QueryResult::Scalar(v) => {
                let mut labels = self.labels.clone();
                labels.insert("__name__", &self.name);
                tsdb.append(labels, Sample::new(ts_ms, v));
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsdb::{Tsdb, TsdbConfig};
    use crate::model::LabelMatcher;

    #[test]
    fn test_recording_rule() {
        let tsdb = Arc::new(Tsdb::default());
        // Seed data
        tsdb.append(
            Labels::from_pairs([("__name__", "http_requests"), ("job", "api")]),
            Sample::new(1000, 10.0),
        );
        tsdb.append(
            Labels::from_pairs([("__name__", "http_requests"), ("job", "api")]),
            Sample::new(2000, 20.0),
        );

        let engine = Engine::new(Arc::clone(&tsdb));
        let rule = RecordingRule::new("job:http_requests:sum", "sum(http_requests)");
        rule.evaluate(&engine, &tsdb, 2000).unwrap();

        // Check the recorded series
        let series = tsdb.select(
            &[LabelMatcher::equal("__name__", "job:http_requests:sum")],
            0, 3000,
        );
        assert!(!series.is_empty());
    }
}

//! Recording rules.

#![allow(dead_code)]

use crate::error::{MetricsError, MetricsResult};
use crate::model::Labels;
use crate::promql::{Engine, EvalContext, QueryValue};
use crate::promql::parser::parse;
use crate::tsdb::Tsdb;

#[derive(Debug, Clone)]
pub struct RecordingRule {
    pub name: String,
    pub expr: String,
    pub labels: Labels,
    pub interval_ms: u64,
}

impl RecordingRule {
    pub async fn evaluate(&self, engine: &Engine, tsdb: &Tsdb, now_ms: i64) -> MetricsResult<()> {
        let expr = parse(&self.expr)?;
        let ctx = EvalContext::instant(now_ms);
        let result = engine.eval_instant(&expr, &ctx, tsdb)?;
        match result {
            QueryValue::InstantVector(samples) => {
                for s in samples {
                    // Build labels: start with rule labels, overlay with series labels, set __name__
                    let mut combined = self.labels.0.clone();
                    for (k, v) in &s.labels.0 {
                        if k != "__name__" {
                            combined.insert(k.clone(), v.clone());
                        }
                    }
                    combined.insert("__name__".to_string(), self.name.clone());
                    let labels = Labels(combined);
                    tsdb.append(labels, now_ms, s.value)?;
                }
            }
            QueryValue::Scalar(v) => {
                let mut lbls = self.labels.0.clone();
                lbls.insert("__name__".to_string(), self.name.clone());
                tsdb.append(Labels(lbls), now_ms, v)?;
            }
            _ => return Err(MetricsError::Eval("recording rule must return vector or scalar".to_string())),
        }
        Ok(())
    }
}

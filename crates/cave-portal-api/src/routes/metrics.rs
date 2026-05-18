// SPDX-License-Identifier: AGPL-3.0-or-later
//! Metrics — point-in-time series query API.
//!
//! Backstage's monitoring tab is one of the most valuable upstream views; the
//! cave portal renders metrics natively and never hands the user off to a
//! Grafana URL. This module exposes the data layer the renderer reads from.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp: u64,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesId {
    pub tenant: String,
    pub name: String,
    pub labels: Vec<(String, String)>,
}

impl SeriesId {
    pub fn key(&self) -> String {
        let mut k = format!("{}::{}", self.tenant, self.name);
        let mut labs = self.labels.clone();
        labs.sort();
        for (l, v) in labs {
            k.push_str(&format!(":{l}={v}"));
        }
        k
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub id: SeriesId,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetricQuery {
    pub tenant: String,
    pub name: String,
    #[serde(default)]
    pub from_ts: Option<u64>,
    #[serde(default)]
    pub to_ts: Option<u64>,
    #[serde(default)]
    pub label_match: Vec<(String, String)>,
    #[serde(default)]
    pub aggregate: Option<Aggregate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    Sum,
    Avg,
    Min,
    Max,
    Count,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MetricsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("invalid range: from={from} to={to}")]
    InvalidRange { from: u64, to: u64 },
    #[error("series not found")]
    NotFound,
    #[error("no samples in range")]
    Empty,
}

pub struct MetricsStore {
    series: Mutex<HashMap<String, Series>>,
}

impl Default for MetricsStore {
    fn default() -> Self {
        Self { series: Mutex::new(HashMap::new()) }
    }
}

impl MetricsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(
        &self,
        principal: Option<&Principal>,
        id: SeriesId,
        sample: Sample,
    ) -> Result<(), MetricsError> {
        Guard::operator_only().authorize(principal, None)?;
        let key = id.key();
        let mut g = self.series.lock().unwrap();
        let entry = g.entry(key).or_insert_with(|| Series {
            id: id.clone(),
            samples: Vec::new(),
        });
        entry.samples.push(sample);
        Ok(())
    }

    pub fn query(
        &self,
        principal: Option<&Principal>,
        q: &MetricQuery,
    ) -> Result<Vec<Series>, MetricsError> {
        Guard::cross_persona(None).authorize(principal, Some(&q.tenant))?;
        if let (Some(f), Some(t)) = (q.from_ts, q.to_ts) {
            if f > t {
                return Err(MetricsError::InvalidRange { from: f, to: t });
            }
        }
        let g = self.series.lock().unwrap();
        let mut out: Vec<Series> = g
            .values()
            .filter(|s| s.id.tenant == q.tenant && s.id.name == q.name)
            .filter(|s| {
                for (lk, lv) in &q.label_match {
                    let matches = s.id.labels.iter().any(|(k, v)| k == lk && v == lv);
                    if !matches {
                        return false;
                    }
                }
                true
            })
            .map(|s| {
                let samples: Vec<Sample> = s
                    .samples
                    .iter()
                    .filter(|smp| {
                        if let Some(f) = q.from_ts {
                            if smp.timestamp < f {
                                return false;
                            }
                        }
                        if let Some(t) = q.to_ts {
                            if smp.timestamp > t {
                                return false;
                            }
                        }
                        true
                    })
                    .cloned()
                    .collect();
                Series { id: s.id.clone(), samples }
            })
            .collect();
        out.sort_by(|a, b| a.id.key().cmp(&b.id.key()));
        Ok(out)
    }

    pub fn aggregate(
        &self,
        principal: Option<&Principal>,
        q: &MetricQuery,
        agg: Aggregate,
    ) -> Result<f64, MetricsError> {
        let series = self.query(principal, q)?;
        let values: Vec<f64> = series
            .iter()
            .flat_map(|s| s.samples.iter().map(|s| s.value))
            .collect();
        if values.is_empty() {
            return Err(MetricsError::Empty);
        }
        Ok(match agg {
            Aggregate::Sum => values.iter().sum(),
            Aggregate::Avg => values.iter().sum::<f64>() / values.len() as f64,
            Aggregate::Min => values.iter().cloned().fold(f64::INFINITY, f64::min),
            Aggregate::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            Aggregate::Count => values.len() as f64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;

    fn op() -> Principal { Principal::new("o", Persona::Operator) }
    fn dev(t: &str) -> Principal { Principal::new("d", Persona::Tenant).with_tenant(t) }

    fn series(tenant: &str, name: &str, labels: &[(&str, &str)]) -> SeriesId {
        SeriesId {
            tenant: tenant.into(),
            name: name.into(),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    fn sample(ts: u64, v: f64) -> Sample {
        Sample { timestamp: ts, value: v }
    }

    #[test]
    fn series_key_includes_labels_sorted() {
        let id1 = series("t", "n", &[("a", "1"), ("b", "2")]);
        let id2 = series("t", "n", &[("b", "2"), ("a", "1")]);
        assert_eq!(id1.key(), id2.key());
    }

    #[test]
    fn record_anonymous_denied() {
        let s = MetricsStore::new();
        let err = s.record(None, series("t", "n", &[]), sample(0, 0.0)).unwrap_err();
        assert!(matches!(err, MetricsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn record_tenant_persona_denied() {
        let s = MetricsStore::new();
        let err = s.record(Some(&dev("acme")), series("acme", "n", &[]), sample(0, 0.0)).unwrap_err();
        assert!(matches!(err, MetricsError::Guard(GuardError::PersonaForbidden { .. })));
    }

    #[test]
    fn record_operator_succeeds() {
        let s = MetricsStore::new();
        assert!(s.record(Some(&op()), series("acme", "n", &[]), sample(1, 2.0)).is_ok());
    }

    #[test]
    fn query_returns_recorded_series() {
        let s = MetricsStore::new();
        s.record(Some(&op()), series("acme", "cpu", &[]), sample(1, 0.5)).unwrap();
        s.record(Some(&op()), series("acme", "cpu", &[]), sample(2, 0.6)).unwrap();
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].samples.len(), 2);
    }

    #[test]
    fn query_filters_by_tenant() {
        let s = MetricsStore::new();
        s.record(Some(&op()), series("acme", "cpu", &[]), sample(1, 0.5)).unwrap();
        s.record(Some(&op()), series("globex", "cpu", &[]), sample(1, 0.9)).unwrap();
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].samples[0].value, 0.5);
    }

    #[test]
    fn query_dev_cross_tenant_denied() {
        let s = MetricsStore::new();
        s.record(Some(&op()), series("acme", "cpu", &[]), sample(1, 0.5)).unwrap();
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let err = s.query(Some(&dev("globex")), &q).unwrap_err();
        assert!(matches!(err, MetricsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn query_filters_by_label() {
        let s = MetricsStore::new();
        s.record(Some(&op()), series("acme", "cpu", &[("host", "h1")]), sample(1, 0.5)).unwrap();
        s.record(Some(&op()), series("acme", "cpu", &[("host", "h2")]), sample(1, 0.9)).unwrap();
        let q = MetricQuery {
            tenant: "acme".into(),
            name: "cpu".into(),
            label_match: vec![("host".into(), "h1".into())],
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn query_filters_time_range() {
        let s = MetricsStore::new();
        let id = series("acme", "cpu", &[]);
        for ts in 1..=5 {
            s.record(Some(&op()), id.clone(), sample(ts, ts as f64)).unwrap();
        }
        let q = MetricQuery {
            tenant: "acme".into(),
            name: "cpu".into(),
            from_ts: Some(2),
            to_ts: Some(4),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out[0].samples.len(), 3);
    }

    #[test]
    fn query_invalid_range_rejected() {
        let s = MetricsStore::new();
        let q = MetricQuery {
            tenant: "acme".into(),
            name: "cpu".into(),
            from_ts: Some(10),
            to_ts: Some(1),
            ..Default::default()
        };
        let err = s.query(Some(&dev("acme")), &q).unwrap_err();
        assert!(matches!(err, MetricsError::InvalidRange { .. }));
    }

    #[test]
    fn aggregate_sum() {
        let s = MetricsStore::new();
        let id = series("acme", "cpu", &[]);
        s.record(Some(&op()), id.clone(), sample(1, 1.0)).unwrap();
        s.record(Some(&op()), id.clone(), sample(2, 2.0)).unwrap();
        s.record(Some(&op()), id.clone(), sample(3, 3.0)).unwrap();
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let v = s.aggregate(Some(&dev("acme")), &q, Aggregate::Sum).unwrap();
        assert!((v - 6.0).abs() < 1e-9);
    }

    #[test]
    fn aggregate_avg() {
        let s = MetricsStore::new();
        let id = series("acme", "cpu", &[]);
        s.record(Some(&op()), id.clone(), sample(1, 2.0)).unwrap();
        s.record(Some(&op()), id.clone(), sample(2, 4.0)).unwrap();
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let v = s.aggregate(Some(&dev("acme")), &q, Aggregate::Avg).unwrap();
        assert!((v - 3.0).abs() < 1e-9);
    }

    #[test]
    fn aggregate_min_max_count() {
        let s = MetricsStore::new();
        let id = series("acme", "cpu", &[]);
        for v in [3.0, 1.0, 7.0, 2.0] {
            s.record(Some(&op()), id.clone(), sample(0, v)).unwrap();
        }
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let p = dev("acme");
        assert_eq!(s.aggregate(Some(&p), &q, Aggregate::Min).unwrap(), 1.0);
        assert_eq!(s.aggregate(Some(&p), &q, Aggregate::Max).unwrap(), 7.0);
        assert_eq!(s.aggregate(Some(&p), &q, Aggregate::Count).unwrap(), 4.0);
    }

    #[test]
    fn aggregate_empty_errors() {
        let s = MetricsStore::new();
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let err = s.aggregate(Some(&dev("acme")), &q, Aggregate::Sum).unwrap_err();
        assert_eq!(err, MetricsError::Empty);
    }

    #[test]
    fn query_returns_sorted_by_series_key() {
        let s = MetricsStore::new();
        s.record(Some(&op()), series("acme", "cpu", &[("z", "1")]), sample(0, 0.0)).unwrap();
        s.record(Some(&op()), series("acme", "cpu", &[("a", "1")]), sample(0, 0.0)).unwrap();
        let q = MetricQuery { tenant: "acme".into(), name: "cpu".into(), ..Default::default() };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        let keys: Vec<String> = out.iter().map(|s| s.id.key()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
}

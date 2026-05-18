// SPDX-License-Identifier: AGPL-3.0-or-later
//! OTLP metrics ingestion (JSON over HTTP; gRPC support uses the same encoding).
//! We parse the OTLP JSON representation and convert to our internal model.

use serde::Deserialize;
use crate::error::{MetricsError, Result};
use crate::model::{Labels, Sample, TimeSeries};
use super::IngestedBatch;

// ─── OTLP JSON schema (minimal subset needed for metrics) ───────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMetricsServiceRequest {
    pub resource_metrics: Vec<ResourceMetrics>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetrics {
    pub resource: Option<Resource>,
    pub scope_metrics: Vec<ScopeMetrics>,
}

#[derive(Debug, Deserialize)]
pub struct Resource {
    pub attributes: Vec<KeyValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeMetrics {
    pub metrics: Vec<Metric>,
}

#[derive(Debug, Deserialize)]
pub struct Metric {
    pub name: String,
    pub description: Option<String>,
    pub unit: Option<String>,
    pub sum: Option<Sum>,
    pub gauge: Option<Gauge>,
    pub histogram: Option<Histogram>,
    pub summary: Option<Summary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sum {
    pub data_points: Vec<NumberDataPoint>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gauge {
    pub data_points: Vec<NumberDataPoint>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Histogram {
    pub data_points: Vec<HistogramDataPoint>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub data_points: Vec<SummaryDataPoint>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberDataPoint {
    pub attributes: Option<Vec<KeyValue>>,
    pub time_unix_nano: Option<String>,
    pub start_time_unix_nano: Option<String>,
    pub as_double: Option<f64>,
    pub as_int: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistogramDataPoint {
    pub attributes: Option<Vec<KeyValue>>,
    pub time_unix_nano: Option<String>,
    pub count: Option<u64>,
    pub sum: Option<f64>,
    pub bucket_counts: Option<Vec<u64>>,
    pub explicit_bounds: Option<Vec<f64>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryDataPoint {
    pub attributes: Option<Vec<KeyValue>>,
    pub time_unix_nano: Option<String>,
    pub count: Option<u64>,
    pub sum: Option<f64>,
    pub quantile_values: Option<Vec<ValueAtQuantile>>,
}

#[derive(Debug, Deserialize)]
pub struct ValueAtQuantile {
    pub quantile: f64,
    pub value: f64,
}

#[derive(Debug, Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: AnyValue,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnyValue {
    Object { #[serde(rename = "stringValue")] string_value: Option<String> },
    String(String),
    Number(f64),
    Bool(bool),
}

impl AnyValue {
    pub fn as_string(&self) -> String {
        match self {
            AnyValue::Object { string_value } => string_value.clone().unwrap_or_default(),
            AnyValue::String(s) => s.clone(),
            AnyValue::Number(n) => n.to_string(),
            AnyValue::Bool(b)   => b.to_string(),
        }
    }
}

// ─── Conversion ──────────────────────────────────────────────────────────────

/// Parse OTLP JSON into our internal batch.
pub fn parse_json(body: &str) -> Result<IngestedBatch> {
    let req: ExportMetricsServiceRequest = serde_json::from_str(body)
        .map_err(|e| MetricsError::Ingestion(format!("OTLP JSON parse: {}", e)))?;
    Ok(convert(req))
}

pub fn convert(req: ExportMetricsServiceRequest) -> IngestedBatch {
    let mut batch = Vec::new();

    for rm in req.resource_metrics {
        let resource_attrs = resource_labels(rm.resource.as_ref());

        for sm in rm.scope_metrics {
            for metric in sm.metrics {
                let name = &metric.name;

                if let Some(sum) = metric.sum {
                    for dp in sum.data_points {
                        batch.push(dp_to_ts(name, &dp, &resource_attrs));
                    }
                }
                if let Some(gauge) = metric.gauge {
                    for dp in gauge.data_points {
                        batch.push(dp_to_ts(name, &dp, &resource_attrs));
                    }
                }
                if let Some(hist) = metric.histogram {
                    for dp in hist.data_points {
                        batch.extend(hist_dp_to_ts(name, &dp, &resource_attrs));
                    }
                }
                if let Some(summary) = metric.summary {
                    for dp in summary.data_points {
                        batch.extend(summary_dp_to_ts(name, &dp, &resource_attrs));
                    }
                }
            }
        }
    }

    batch
}

fn resource_labels(resource: Option<&Resource>) -> Vec<(String, String)> {
    resource.map(|r| r.attributes.iter().map(|kv| (kv.key.clone(), kv.value.as_string())).collect())
        .unwrap_or_default()
}

fn attr_labels(attrs: Option<&Vec<KeyValue>>) -> Vec<(String, String)> {
    attrs.map(|a| a.iter().map(|kv| (kv.key.clone(), kv.value.as_string())).collect())
        .unwrap_or_default()
}

fn nano_to_ms(nano: Option<&String>) -> i64 {
    nano.and_then(|s| s.parse::<u64>().ok())
        .map(|n| (n / 1_000_000) as i64)
        .unwrap_or_else(|| now_ms())
}

fn dp_to_ts(name: &str, dp: &NumberDataPoint, resource_attrs: &[(String, String)]) -> TimeSeries {
    let mut labels = Labels::from_pairs(resource_attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    for (k, v) in attr_labels(dp.attributes.as_ref()) {
        labels.insert(k, v);
    }
    labels.insert("__name__", name);

    let value = dp.as_double.unwrap_or_else(|| dp.as_int.map(|i| i as f64).unwrap_or(f64::NAN));
    let ts_ms = nano_to_ms(dp.time_unix_nano.as_ref());

    TimeSeries { labels, samples: vec![Sample::new(ts_ms, value)] }
}

fn hist_dp_to_ts(name: &str, dp: &HistogramDataPoint, resource_attrs: &[(String, String)]) -> Vec<TimeSeries> {
    let mut base_labels = Labels::from_pairs(resource_attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    for (k, v) in attr_labels(dp.attributes.as_ref()) {
        base_labels.insert(k, v);
    }
    let ts_ms = nano_to_ms(dp.time_unix_nano.as_ref());
    let mut out = Vec::new();

    // _count
    if let Some(count) = dp.count {
        let mut l = base_labels.clone();
        l.insert("__name__", format!("{}_count", name));
        out.push(TimeSeries { labels: l, samples: vec![Sample::new(ts_ms, count as f64)] });
    }

    // _sum
    if let Some(sum) = dp.sum {
        let mut l = base_labels.clone();
        l.insert("__name__", format!("{}_sum", name));
        out.push(TimeSeries { labels: l, samples: vec![Sample::new(ts_ms, sum)] });
    }

    // _bucket
    if let (Some(bounds), Some(counts)) = (&dp.explicit_bounds, &dp.bucket_counts) {
        let mut cumulative = 0u64;
        for (i, &bound) in bounds.iter().enumerate() {
            cumulative += counts.get(i).copied().unwrap_or(0);
            let mut l = base_labels.clone();
            l.insert("__name__", format!("{}_bucket", name));
            l.insert("le", bound.to_string());
            out.push(TimeSeries { labels: l, samples: vec![Sample::new(ts_ms, cumulative as f64)] });
        }
        // +Inf bucket
        cumulative += counts.get(bounds.len()).copied().unwrap_or(0);
        let mut l = base_labels.clone();
        l.insert("__name__", format!("{}_bucket", name));
        l.insert("le", "+Inf");
        out.push(TimeSeries { labels: l, samples: vec![Sample::new(ts_ms, cumulative as f64)] });
    }

    out
}

fn summary_dp_to_ts(name: &str, dp: &SummaryDataPoint, resource_attrs: &[(String, String)]) -> Vec<TimeSeries> {
    let mut base_labels = Labels::from_pairs(resource_attrs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    for (k, v) in attr_labels(dp.attributes.as_ref()) {
        base_labels.insert(k, v);
    }
    let ts_ms = nano_to_ms(dp.time_unix_nano.as_ref());
    let mut out = Vec::new();

    if let Some(count) = dp.count {
        let mut l = base_labels.clone();
        l.insert("__name__", format!("{}_count", name));
        out.push(TimeSeries { labels: l, samples: vec![Sample::new(ts_ms, count as f64)] });
    }
    if let Some(sum) = dp.sum {
        let mut l = base_labels.clone();
        l.insert("__name__", format!("{}_sum", name));
        out.push(TimeSeries { labels: l, samples: vec![Sample::new(ts_ms, sum)] });
    }
    if let Some(qs) = &dp.quantile_values {
        for q in qs {
            let mut l = base_labels.clone();
            l.insert("__name__", name);
            l.insert("quantile", q.quantile.to_string());
            out.push(TimeSeries { labels: l, samples: vec![Sample::new(ts_ms, q.value)] });
        }
    }

    out
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

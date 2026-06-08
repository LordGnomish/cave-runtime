// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus scaler — scales on a PromQL query result.
//! upstream: kedacore/keda v2.x — pkg/scalers/prometheus_scaler.go

use crate::scaler::ScalerTrait;
use serde::Deserialize;
use std::time::Duration;

/// Prometheus `/api/v1/query` response envelope — the subset
/// prometheus_scaler.go's `promQueryResult` consumes for scalar extraction.
#[derive(Debug, Deserialize)]
struct PromQueryResult {
    data: PromData,
}

#[derive(Debug, Deserialize)]
struct PromData {
    #[serde(default)]
    result: Vec<PromResultEntry>,
}

#[derive(Debug, Deserialize)]
struct PromResultEntry {
    /// Instant-vector sample: `[ <unix_ts: number>, "<value: string>" ]`.
    #[serde(default)]
    value: Vec<serde_json::Value>,
}

/// Port of prometheus_scaler.go `ExecutePromQuery`'s result handling: extract
/// the scalar metric value from a Prometheus `/api/v1/query` response body.
///
/// Faithful to upstream semantics:
/// * empty result set → `0` when `ignore_null_values`, else an error
///   ("the result is empty");
/// * more than one element → error ("returned multiple elements");
/// * a sample value list with no entries → `0`/error as above; with a single
///   entry → "didn't return enough values";
/// * otherwise `value[1]` (the metric value string) is parsed as `f64`. A JSON
///   `null` value yields KEDA's `var v float64 = -1` default.
pub fn parse_prom_query_result(body: &str, ignore_null_values: bool) -> Result<f64, String> {
    let result: PromQueryResult = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let results = &result.data.result;
    if results.is_empty() {
        if ignore_null_values {
            return Ok(0.0);
        }
        return Err(
            "prometheus metrics 'prometheus' target may be lost, the result is empty".into(),
        );
    } else if results.len() > 1 {
        return Err("prometheus query returned multiple elements".into());
    }

    let value = &results[0].value;
    match value.len() {
        0 => {
            if ignore_null_values {
                Ok(0.0)
            } else {
                Err(
                    "prometheus metrics 'prometheus' target may be lost, the value list is empty"
                        .into(),
                )
            }
        }
        1 => Err("prometheus query didn't return enough values".into()),
        _ => match &value[1] {
            // KEDA leaves the `var v float64 = -1` default when value[1] is nil.
            serde_json::Value::Null => Ok(-1.0),
            serde_json::Value::String(s) => s
                .parse::<f64>()
                .map_err(|e| format!("error converting prometheus value '{s}': {e}")),
            other => Err(format!("unexpected prometheus value type: {other}")),
        },
    }
}

#[derive(Default, Debug, Clone)]
pub struct PrometheusScaler {
    pub tenant_id: String,
    pub server_address: String,
    pub query: String,
    pub threshold: f64,
    pub activation_threshold: f64,
    pub current_value: f64,
}

impl PrometheusScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            server_address: String::new(),
            query: String::new(),
            threshold: 100.0,
            activation_threshold: 0.0,
            current_value: 0.0,
        }
    }

    pub fn observe(&mut self, value: f64) {
        self.current_value = if value.is_nan() { 0.0 } else { value };
    }

    /// Parse a Prometheus `/api/v1/query` response body and record the scalar
    /// result as the current metric value (the userspace half of
    /// `ExecutePromQuery`). Returns the parse error verbatim on failure.
    pub fn observe_query_response(
        &mut self,
        body: &str,
        ignore_null_values: bool,
    ) -> Result<(), String> {
        let v = parse_prom_query_result(body, ignore_null_values)?;
        self.observe(v);
        Ok(())
    }
}

impl ScalerTrait for PrometheusScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_value)
    }

    fn is_active(&self) -> bool {
        self.current_value > self.activation_threshold
    }

    fn activation_threshold(&self) -> f64 {
        self.activation_threshold
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE: &str = r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"__name__":"http_requests"},"value":[1700000000.5,"42.7"]}]}}"#;
    const EMPTY: &str = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;
    const MULTI: &str = r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1.0,"1"]},{"metric":{},"value":[1.0,"2"]}]}}"#;

    #[test]
    fn parses_single_vector_value() {
        assert_eq!(parse_prom_query_result(SINGLE, false).unwrap(), 42.7);
    }

    #[test]
    fn empty_result_honors_ignore_null_values() {
        assert_eq!(parse_prom_query_result(EMPTY, true).unwrap(), 0.0);
        assert!(parse_prom_query_result(EMPTY, false).is_err());
    }

    #[test]
    fn multiple_elements_is_an_error() {
        assert!(parse_prom_query_result(MULTI, true).is_err());
    }

    #[test]
    fn value_list_too_short_is_an_error() {
        let short = r#"{"data":{"result":[{"value":[1700000000.0]}]}}"#;
        assert!(parse_prom_query_result(short, false).is_err());
    }

    #[test]
    fn null_value_yields_keda_minus_one_default() {
        let nullv = r#"{"data":{"result":[{"value":[1700000000.0,null]}]}}"#;
        assert_eq!(parse_prom_query_result(nullv, false).unwrap(), -1.0);
    }

    #[test]
    fn observe_query_response_updates_metric_and_activation() {
        let mut s = PrometheusScaler::new("t1");
        s.activation_threshold = 10.0;
        s.observe_query_response(SINGLE, false).unwrap();
        assert_eq!(s.metric_value(), Some(42.7));
        assert!(s.is_active()); // 42.7 > 10
    }

    #[test]
    fn observe_query_response_propagates_parse_error() {
        let mut s = PrometheusScaler::new("t1");
        assert!(s.observe_query_response(EMPTY, false).is_err());
    }
}

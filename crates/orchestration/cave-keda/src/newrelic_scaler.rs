// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! New Relic scaler — NRQL query-result scaler.
//!
//! Faithful line-port of kedacore/keda v2.16.1 pkg/scalers/newrelic_scaler.go:
//!   - `executeNewRelicQuery` result extraction:
//!       * empty result set → error iff `noDataError`, else 0
//!       * otherwise scan the first result row and return the first value that
//!         is a float; if none is a float → error iff `noDataError`, else 0
//!   - `parseNewRelicMetadata` default region "US"
//!   - `GetMetricsAndActivity` activation gate (`val > activationThreshold`)
//!
//! Only the pure decision logic is ported here; the live NRDB query is handled
//! by the runtime transport layer.

use crate::scaler::ScalerTrait;
use std::time::Duration;

/// One cell of an NRDB result row. NRDB returns `map[string]interface{}`; we
/// model the heterogeneous cell as a tagged enum, ordered within a row so that
/// "first float" selection is deterministic.
#[derive(Debug, Clone, PartialEq)]
pub enum NrdbValue {
    Float(f64),
    Str(String),
    Bool(bool),
}

impl NrdbValue {
    fn as_float(&self) -> Option<f64> {
        match self {
            NrdbValue::Float(v) => Some(*v),
            _ => None,
        }
    }
}

/// New Relic NRQL scaler.
#[derive(Debug, Clone)]
pub struct NewRelicScaler {
    pub account: i64,
    pub nrql: String,
    pub region: String,
    pub no_data_error: bool,
    pub threshold: f64,
    pub activation_threshold: f64,
    current_value: f64,
}

impl NewRelicScaler {
    pub fn new(account: i64, nrql: &str) -> Self {
        Self {
            account,
            nrql: nrql.to_string(),
            // upstream: region defaults to "US" when absent.
            region: "US".to_string(),
            no_data_error: false,
            threshold: 0.0,
            activation_threshold: 0.0,
            current_value: 0.0,
        }
    }

    /// Port of `executeNewRelicQuery` result handling.
    ///
    /// ```go
    /// if len(resp.Results) == 0 {
    ///     if s.metadata.noDataError { return 0, fmt.Errorf("query return no results %s", ...) }
    ///     return 0, nil
    /// }
    /// for _, v := range resp.Results[0] {
    ///     val, ok := v.(float64)
    ///     if ok { return val, nil }
    /// }
    /// if s.metadata.noDataError { return 0, fmt.Errorf("query return no results %s", ...) }
    /// return 0, nil
    /// ```
    pub fn extract_metric(
        results: &[Vec<NrdbValue>],
        no_data_error: bool,
    ) -> Result<f64, String> {
        if results.is_empty() {
            if no_data_error {
                return Err("query return no results".to_string());
            }
            return Ok(0.0);
        }
        for v in &results[0] {
            if let Some(f) = v.as_float() {
                return Ok(f);
            }
        }
        if no_data_error {
            return Err("query return no results".to_string());
        }
        Ok(0.0)
    }

    /// Record a freshly-fetched query value.
    pub fn observe(&mut self, value: f64) {
        self.current_value = if value.is_nan() { 0.0 } else { value };
    }
}

impl ScalerTrait for NewRelicScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_value)
    }

    /// Port of `GetMetricsAndActivity`: `val > s.metadata.activationThreshold`.
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

    #[test]
    fn extract_skips_non_float_cells() {
        let rows = vec![vec![
            NrdbValue::Bool(true),
            NrdbValue::Str("x".into()),
            NrdbValue::Float(4.0),
        ]];
        assert_eq!(NewRelicScaler::extract_metric(&rows, false).unwrap(), 4.0);
    }
}

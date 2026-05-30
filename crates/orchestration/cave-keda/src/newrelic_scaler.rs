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
use std::collections::HashMap;
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

    /// Port of `parseNewRelicMetadata`.
    ///
    /// Mirrors the typed-config struct tags: `account` (int) and `nrql` come
    /// from triggerMetadata/authParams and are required; `threshold` (float) is
    /// required for a scaling trigger; `region` defaults to `"US"`;
    /// `activationThreshold` defaults to `0`; `noDataError` is an optional bool.
    /// `queryKey` lives in authParams and is resolved by the transport layer, so
    /// it is not part of the pure-decision metadata here.
    pub fn from_metadata(meta: &HashMap<String, String>) -> Result<Self, String> {
        let account = meta
            .get("account")
            .ok_or_else(|| "account is required".to_string())?
            .parse::<i64>()
            .map_err(|_| "account must be an integer".to_string())?;
        let nrql = meta
            .get("nrql")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "nrql is required".to_string())?
            .clone();
        let threshold = meta
            .get("threshold")
            .ok_or_else(|| "threshold is required".to_string())?
            .parse::<f64>()
            .map_err(|_| "threshold must be a float".to_string())?;

        let region = meta
            .get("region")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "US".to_string());
        let activation_threshold = match meta.get("activationThreshold") {
            Some(v) => v
                .parse::<f64>()
                .map_err(|_| "activationThreshold must be a float".to_string())?,
            None => 0.0,
        };
        let no_data_error = match meta.get("noDataError") {
            Some(v) => v
                .parse::<bool>()
                .map_err(|_| "noDataError must be a bool".to_string())?,
            None => false,
        };

        Ok(Self {
            account,
            nrql,
            region,
            no_data_error,
            threshold,
            activation_threshold,
            current_value: 0.0,
        })
    }

    /// Port of the scaler metric name: `scalerName = "new-relic"`, normalized.
    pub fn metric_name(&self) -> String {
        "new-relic".to_string()
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

    fn md(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn from_metadata_parses_required_fields_and_defaults() {
        let s = NewRelicScaler::from_metadata(&md(&[
            ("account", "12345"),
            ("nrql", "SELECT count(*) FROM Transaction"),
            ("threshold", "100"),
        ]))
        .unwrap();
        assert_eq!(s.account, 12345);
        assert_eq!(s.nrql, "SELECT count(*) FROM Transaction");
        assert_eq!(s.threshold, 100.0);
        // region defaults to "US" when absent (parseNewRelicMetadata default).
        assert_eq!(s.region, "US");
        // activationThreshold default=0; noDataError optional default false.
        assert_eq!(s.activation_threshold, 0.0);
        assert!(!s.no_data_error);
    }

    #[test]
    fn from_metadata_honours_explicit_overrides() {
        let s = NewRelicScaler::from_metadata(&md(&[
            ("account", "7"),
            ("nrql", "SELECT 1"),
            ("threshold", "5"),
            ("region", "EU"),
            ("activationThreshold", "2.5"),
            ("noDataError", "true"),
        ]))
        .unwrap();
        assert_eq!(s.region, "EU");
        assert_eq!(s.activation_threshold, 2.5);
        assert!(s.no_data_error);
    }

    #[test]
    fn from_metadata_requires_account_nrql_threshold() {
        assert!(NewRelicScaler::from_metadata(&md(&[("nrql", "x"), ("threshold", "1")])).is_err());
        assert!(
            NewRelicScaler::from_metadata(&md(&[("account", "1"), ("threshold", "1")])).is_err()
        );
        assert!(NewRelicScaler::from_metadata(&md(&[("account", "1"), ("nrql", "x")])).is_err());
        // non-integer account is rejected.
        assert!(
            NewRelicScaler::from_metadata(&md(&[
                ("account", "x"),
                ("nrql", "y"),
                ("threshold", "1")
            ]))
            .is_err()
        );
    }

    #[test]
    fn metric_name_is_normalized_scaler_name() {
        let s = NewRelicScaler::new(1, "SELECT 1");
        assert_eq!(s.metric_name(), "new-relic");
    }
}

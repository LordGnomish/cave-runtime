// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dynatrace scaler — Metric Data Points API scaler.
//!
//! Faithful line-port of kedacore/keda v2.16.1 pkg/scalers/dynatrace_scaler.go:
//!   - `dynatraceResponse` model (`Result[].Data[].Values[]`)
//!   - `validateDynatraceResponse` (three-layer nested structure validation
//!     with distinct error messages)
//!   - value extraction (`Result[0].Data[0].Values[0]`)
//!   - `GetMetricValue` URL building (trailing-slash trim + API path append +
//!     `metricSelector`/`from` query params; default `from=now-2h`)
//!   - `GetMetricsAndActivity` activation gate (`val > ActivationThreshold`)
//!
//! Only the pure decision logic is ported here; the live HTTP query is handled
//! by the runtime transport layer.

use crate::scaler::ScalerTrait;
use std::time::Duration;

const DYNATRACE_METRIC_DATA_POINTS_API: &str = "api/v2/metrics/query";

/// One metric series — `Data` element of a Dynatrace result entry.
///
/// upstream model (pkg/scalers/dynatrace_scaler.go):
/// ```go
/// Result []struct {
///     Data []struct {
///         Values []float64 `json:"values"`
///     } `json:"data"`
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct DynatraceSeries {
    /// Each element is the `Values` array of a single data series.
    pub data: Vec<Vec<f64>>,
}

/// Relevant subset of Dynatrace's Metric Data Points API response.
#[derive(Debug, Clone, Default)]
pub struct DynatraceResponse {
    pub result: Vec<DynatraceSeries>,
}

impl DynatraceResponse {
    /// Port of `validateDynatraceResponse`.
    ///
    /// ```go
    /// if len(response.Result) == 0 { return errors.New("...any results") }
    /// if len(response.Result[0].Data) == 0 { return errors.New("...any metric series") }
    /// if len(response.Result[0].Data[0].Values) == 0 { return errors.New("...any values...") }
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.result.is_empty() {
            return Err("dynatrace response does not contain any results".to_string());
        }
        if self.result[0].data.is_empty() {
            return Err("dynatrace response does not contain any metric series".to_string());
        }
        if self.result[0].data[0].is_empty() {
            return Err(
                "dynatrace response does not contain any values for the metric series".to_string(),
            );
        }
        Ok(())
    }

    /// Port of the final return of `GetMetricValue`:
    /// `return dynatraceResponse.Result[0].Data[0].Values[0], nil` (after validation).
    pub fn first_value(&self) -> Result<f64, String> {
        self.validate()?;
        Ok(self.result[0].data[0][0])
    }
}

/// Dynatrace metric scaler.
#[derive(Debug, Clone)]
pub struct DynatraceScaler {
    pub host: String,
    pub metric_selector: String,
    pub token: String,
    pub from_timestamp: String,
    pub threshold: f64,
    pub activation_threshold: f64,
    current_value: f64,
}

impl DynatraceScaler {
    pub fn new(host: &str, metric_selector: &str, token: &str) -> Self {
        Self {
            host: host.to_string(),
            metric_selector: metric_selector.to_string(),
            token: token.to_string(),
            // upstream default: `keda:"name=from, ... default=now-2h"`
            from_timestamp: "now-2h".to_string(),
            threshold: 0.0,
            activation_threshold: 0.0,
            current_value: 0.0,
        }
    }

    /// Port of the URL-assembly block of `GetMetricValue`:
    /// ```go
    /// dynatraceAPIURL := fmt.Sprintf("%s/%s", strings.TrimRight(s.metadata.Host, "/"),
    ///     dynatraceMetricDataPointsAPI)
    /// queryString.Set("metricSelector", s.metadata.MetricSelector)
    /// queryString.Set("from", s.metadata.FromTimestamp)
    /// ```
    pub fn build_query_url(host: &str, metric_selector: &str, from_timestamp: &str) -> String {
        let base = format!(
            "{}/{}",
            host.trim_end_matches('/'),
            DYNATRACE_METRIC_DATA_POINTS_API
        );
        format!(
            "{base}?metricSelector={}&from={}",
            url_query_encode(metric_selector),
            url_query_encode(from_timestamp)
        )
    }

    /// Record a freshly-fetched metric value.
    pub fn observe(&mut self, value: f64) {
        self.current_value = if value.is_nan() { 0.0 } else { value };
    }
}

impl ScalerTrait for DynatraceScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_value)
    }

    /// Port of `GetMetricsAndActivity`: `val > s.metadata.ActivationThreshold`.
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

/// Minimal application/x-www-form-urlencoded query-component encoder, matching
/// the subset of `net/url` Values.Encode used by the upstream query string
/// (percent-encode all non-unreserved bytes; unreserved = ALPHA / DIGIT / `-_.~`).
fn url_query_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_percent_encodes_colon() {
        assert_eq!(url_query_encode("a:b"), "a%3Ab");
    }

    #[test]
    fn first_value_picks_leading_datapoint() {
        let r = DynatraceResponse {
            result: vec![DynatraceSeries {
                data: vec![vec![1.5, 2.5]],
            }],
        };
        assert_eq!(r.first_value().unwrap(), 1.5);
    }

    fn md(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn from_metadata_parses_required_and_defaults() {
        let s = DynatraceScaler::from_metadata(&md(&[
            ("host", "https://abc.live.dynatrace.com"),
            ("metricSelector", "builtin:service.requestCount.total"),
            ("token", "dt0c01.SECRET"),
            ("threshold", "1000"),
        ]))
        .unwrap();
        assert_eq!(s.host, "https://abc.live.dynatrace.com");
        assert_eq!(s.metric_selector, "builtin:service.requestCount.total");
        assert_eq!(s.token, "dt0c01.SECRET");
        assert_eq!(s.threshold, 1000.0);
        // from defaults to now-2h; activationThreshold optional → 0.
        assert_eq!(s.from_timestamp, "now-2h");
        assert_eq!(s.activation_threshold, 0.0);
    }

    #[test]
    fn from_metadata_overrides_from_and_activation() {
        let s = DynatraceScaler::from_metadata(&md(&[
            ("host", "https://x"),
            ("metricSelector", "m"),
            ("token", "t"),
            ("threshold", "5"),
            ("from", "now-30m"),
            ("activationThreshold", "3"),
        ]))
        .unwrap();
        assert_eq!(s.from_timestamp, "now-30m");
        assert_eq!(s.activation_threshold, 3.0);
    }

    #[test]
    fn from_metadata_requires_host_selector_token_threshold() {
        assert!(
            DynatraceScaler::from_metadata(&md(&[
                ("metricSelector", "m"),
                ("token", "t"),
                ("threshold", "1")
            ]))
            .is_err()
        );
        assert!(
            DynatraceScaler::from_metadata(&md(&[
                ("host", "h"),
                ("token", "t"),
                ("threshold", "1")
            ]))
            .is_err()
        );
        assert!(
            DynatraceScaler::from_metadata(&md(&[
                ("host", "h"),
                ("metricSelector", "m"),
                ("threshold", "1")
            ]))
            .is_err()
        );
        assert!(
            DynatraceScaler::from_metadata(&md(&[
                ("host", "h"),
                ("metricSelector", "m"),
                ("token", "t")
            ]))
            .is_err()
        );
    }

    #[test]
    fn metric_name_is_dynatrace() {
        let s = DynatraceScaler::new("h", "m", "t");
        assert_eq!(s.metric_name(), "dynatrace");
    }
}

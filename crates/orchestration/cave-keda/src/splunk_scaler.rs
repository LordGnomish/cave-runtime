// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Splunk scaler — saved-search result-field scaler.
//!
//! Faithful line-port of kedacore/keda v2.16.1:
//!   - pkg/scalers/splunk/splunk.go
//!       * `SearchResponse.ToMetric` (named value-field extraction + float parse)
//!       * `NewClient` credential validation (username required; APIToken and
//!         Password mutually exclusive)
//!   - pkg/scalers/splunk_scaler.go
//!       * `parseSplunkMetadata` host URL validation
//!       * `GetMetricsAndActivity` activation gate (`int(value) > activationValue`)
//!
//! Only the pure decision logic is ported here; the live HTTP saved-search
//! fetch is handled by the runtime transport layer.

use crate::scaler::ScalerTrait;
use std::collections::HashMap;
use std::time::Duration;

/// Mirrors `splunk.SearchResponse` — the unmarshalled saved-search result.
///
/// upstream: pkg/scalers/splunk/splunk.go
/// ```go
/// type SearchResponse struct {
///     Result map[string]string `json:"result"`
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct SearchResponse {
    /// Field name → stringified value, as returned by Splunk in `output_mode=json`.
    pub result: HashMap<String, String>,
}

impl SearchResponse {
    /// Port of `(*SearchResponse).ToMetric`.
    ///
    /// ```go
    /// metricValueStr, ok := s.Result[valueField]
    /// if !ok { return 0, fmt.Errorf("field: %s not found in search results", valueField) }
    /// metricValueInt, err := strconv.ParseFloat(metricValueStr, 64)
    /// if err != nil { return 0, fmt.Errorf("value: %s is not a float value", valueField) }
    /// return metricValueInt, nil
    /// ```
    pub fn to_metric(&self, value_field: &str) -> Result<f64, String> {
        let metric_value_str = self
            .result
            .get(value_field)
            .ok_or_else(|| format!("field: {value_field} not found in search results"))?;
        metric_value_str
            .parse::<f64>()
            .map_err(|_| format!("value: {value_field} is not a float value"))
    }
}

/// Validation outcomes for the Splunk credential combination, ported from
/// `splunk.NewClient`'s guard clauses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplunkValidationError {
    /// `username was not set`
    UsernameNotSet,
    /// `API token and Password were all set...`
    TokenAndPasswordBothSet,
}

/// Splunk saved-search scaler.
#[derive(Debug, Clone)]
pub struct SplunkScaler {
    pub username: String,
    pub saved_search_name: String,
    pub value_field: String,
    pub target_value: i64,
    pub activation_value: i64,
    current_value: f64,
}

impl SplunkScaler {
    pub fn new(username: &str, saved_search_name: &str, value_field: &str) -> Self {
        Self {
            username: username.to_string(),
            saved_search_name: saved_search_name.to_string(),
            value_field: value_field.to_string(),
            target_value: 1,
            activation_value: 0,
            current_value: 0.0,
        }
    }

    /// Port of `splunk.NewClient` credential guards:
    /// ```go
    /// if c.Username == "" { return nil, errors.New("username was not set") }
    /// if c.APIToken != "" && c.Password != "" {
    ///     return nil, errors.New("API token and Password were all set...")
    /// }
    /// ```
    pub fn validate_credentials(
        username: &str,
        api_token: &str,
        password: &str,
    ) -> Result<(), SplunkValidationError> {
        if username.is_empty() {
            return Err(SplunkValidationError::UsernameNotSet);
        }
        if !api_token.is_empty() && !password.is_empty() {
            return Err(SplunkValidationError::TokenAndPasswordBothSet);
        }
        Ok(())
    }

    /// Port of `parseSplunkMetadata` host validation:
    /// ```go
    /// _, err := url.ParseRequestURI(meta.Host)
    /// if err != nil { return meta, errors.New("invalid value for host...") }
    /// ```
    /// Go's `url.ParseRequestURI` requires an absolute URI (a scheme + an
    /// authority/path). We accept only inputs with a `scheme://` prefix.
    pub fn validate_host(host: &str) -> Result<(), String> {
        let valid = host
            .split_once("://")
            .map(|(scheme, rest)| {
                !scheme.is_empty()
                    && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
                    && !rest.is_empty()
                    && !rest.contains(' ')
            })
            .unwrap_or(false);
        if valid {
            Ok(())
        } else {
            Err(
                "invalid value for host. Must be a valid URL such as https://localhost:8089"
                    .to_string(),
            )
        }
    }

    /// Record a freshly-fetched saved-search metric value.
    pub fn observe(&mut self, value: f64) {
        self.current_value = if value.is_nan() { 0.0 } else { value };
    }
}

impl ScalerTrait for SplunkScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_value)
    }

    /// Port of `GetMetricsAndActivity` activation gate:
    /// `int(metricValue) > s.metadata.ActivationValue`.
    fn is_active(&self) -> bool {
        (self.current_value as i64) > self.activation_value
    }

    fn activation_threshold(&self) -> f64 {
        self.activation_value as f64
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_metric_roundtrips_integer_string() {
        let mut result = HashMap::new();
        result.insert("c".to_string(), "5".to_string());
        let resp = SearchResponse { result };
        assert_eq!(resp.to_metric("c").unwrap(), 5.0);
    }

    #[test]
    fn validate_host_rejects_empty_scheme() {
        assert!(SplunkScaler::validate_host("://localhost").is_err());
    }
}

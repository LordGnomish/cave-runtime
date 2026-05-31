// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scrape-time label and sample limits.
//!
//! Direct port of prometheus/prometheus `scrape/scrape.go` (v3.12.0):
//!   * `labelLimits` / `verifyLabelLimits` → [`LabelLimits`] / [`verify_label_limits`]
//!   * `limitAppender` (sample_limit, stale-marker exempt) → [`SampleLimiter`]
//!
//! In every case a limit of `0` means "disabled", matching upstream.

use crate::error::MetricsError;
use crate::model::Labels;
use crate::Result;

/// Prometheus `value.StaleNaN` — the IEEE-754 quiet-NaN bit pattern used to
/// mark a series stale. Stale markers are exempt from `sample_limit`.
pub const STALE_NAN_BITS: u64 = 0x7ff0_0000_0000_0002;

/// A concrete `f64` carrying the stale-marker bit pattern.
pub const STALE_NAN: f64 = f64::from_bits(STALE_NAN_BITS);

/// True iff `v` is the Prometheus stale marker (the specific NaN payload, not
/// any NaN).
pub fn is_stale_nan(v: f64) -> bool {
    v.to_bits() == STALE_NAN_BITS
}

/// Per-target label limits (`scrape.labelLimits`). `0` disables a limit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelLimits {
    pub label_limit: usize,
    pub label_name_length_limit: usize,
    pub label_value_length_limit: usize,
}

/// `verifyLabelLimits` — reject label sets that breach any configured limit.
pub fn verify_label_limits(lset: &Labels, limits: &LabelLimits) -> Result<()> {
    let met = lset.metric_name().unwrap_or("");

    if limits.label_limit > 0 {
        let nb_labels = lset.iter().count();
        if nb_labels > limits.label_limit {
            return Err(MetricsError::Scrape(format!(
                "label_limit exceeded (metric: {met}, number of labels: {nb_labels}, limit: {})",
                limits.label_limit
            )));
        }
    }

    if limits.label_name_length_limit == 0 && limits.label_value_length_limit == 0 {
        return Ok(());
    }

    for (name, value) in lset.iter() {
        if limits.label_name_length_limit > 0 {
            let name_length = name.len();
            if name_length > limits.label_name_length_limit {
                return Err(MetricsError::Scrape(format!(
                    "label_name_length_limit exceeded (metric: {met}, label name: {name}, length: {name_length}, limit: {})",
                    limits.label_name_length_limit
                )));
            }
        }
        if limits.label_value_length_limit > 0 {
            let value_length = value.len();
            if value_length > limits.label_value_length_limit {
                return Err(MetricsError::Scrape(format!(
                    "label_value_length_limit exceeded (metric: {met}, label name: {name}, value: {value}, length: {value_length}, limit: {})",
                    limits.label_value_length_limit
                )));
            }
        }
    }

    Ok(())
}

/// `limitAppender` — enforce `sample_limit` over one scrape. Stale markers do
/// not count; the running non-stale total may not exceed `limit`. `0` disables.
#[derive(Debug, Clone)]
pub struct SampleLimiter {
    limit: usize,
    count: usize,
}

impl SampleLimiter {
    pub fn new(limit: usize) -> Self {
        Self { limit, count: 0 }
    }

    /// Number of non-stale samples accepted so far.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Account for one sample value. Stale markers are exempt; a non-stale
    /// sample that would push the count past `limit` is rejected.
    pub fn accept(&mut self, value: f64) -> Result<()> {
        if is_stale_nan(value) {
            return Ok(());
        }
        if self.limit == 0 {
            self.count += 1;
            return Ok(());
        }
        self.count += 1;
        if self.count > self.limit {
            return Err(MetricsError::Scrape(format!(
                "per-scrape sample limit exceeded (limit: {})",
                self.limit
            )));
        }
        Ok(())
    }
}

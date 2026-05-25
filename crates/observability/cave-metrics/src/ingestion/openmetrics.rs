// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenMetrics format parser (text/openmetrics-text; version=1.0.0).
//! Superset of Prometheus exposition format with EOF marker and additional types.

use super::IngestedBatch;
use crate::error::Result;
use crate::model::{Labels, Sample, TimeSeries};

/// Parse OpenMetrics text format.
/// Falls back to the Prometheus exposition parser for lines that conform to that format.
pub fn parse(input: &str) -> Result<IngestedBatch> {
    // OpenMetrics is a superset of the Prometheus format.
    // Strip the EOF marker and delegate.
    let stripped = input.trim_end_matches("# EOF").trim_end();
    super::exposition::parse(stripped)
}

/// Check whether a Content-Type header indicates OpenMetrics.
pub fn is_openmetrics(content_type: &str) -> bool {
    content_type.contains("application/openmetrics-text")
        || content_type.contains("text/openmetrics-text")
}

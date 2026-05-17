// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Graphite plaintext protocol parser.
//! Format: <metric.path> <value> <unix_timestamp>\n
//! Also supports the Graphite pickle protocol (skipped here; use plaintext).

use crate::error::{MetricsError, Result};
use crate::model::{Labels, Sample, TimeSeries};
use super::IngestedBatch;

/// Parse a single Graphite plaintext line.
pub fn parse_line(line: &str) -> Result<TimeSeries> {
    let mut parts = line.split_whitespace();
    let path = parts.next().ok_or_else(|| MetricsError::Parse("empty graphite line".into()))?;
    let value_str = parts.next().ok_or_else(|| MetricsError::Parse("missing graphite value".into()))?;
    let ts_str = parts.next();

    let value: f64 = match value_str {
        "nan" | "NaN" => f64::NAN,
        v => v.parse().map_err(|e| MetricsError::Parse(format!("graphite value: {}", e)))?,
    };

    let timestamp_ms = ts_str
        .and_then(|t| t.parse::<i64>().ok())
        .map(|s| s * 1000) // graphite uses Unix seconds
        .unwrap_or_else(now_ms);

    // Convert dotted path to labels.
    // e.g. "servers.web01.cpu.usage" → __name__=servers_web01_cpu_usage, path=servers.web01.cpu.usage
    let metric_name = sanitize_path(path);
    let mut labels = Labels::new();
    labels.insert("__name__", metric_name);
    labels.insert("graphite_path", path);

    Ok(TimeSeries { labels, samples: vec![Sample::new(timestamp_ms, value)] })
}

/// Parse a multi-line Graphite plaintext batch.
pub fn parse_batch(input: &str) -> IngestedBatch {
    input.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| parse_line(l).ok())
        .collect()
}

fn sanitize_path(path: &str) -> String {
    path.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let ts = parse_line("servers.web01.cpu.usage 0.85 1609459200").unwrap();
        assert_eq!(ts.labels.get("__name__"), Some("servers_web01_cpu_usage"));
        assert_eq!(ts.labels.get("graphite_path"), Some("servers.web01.cpu.usage"));
        assert_eq!(ts.samples[0].value, 0.85);
        assert_eq!(ts.samples[0].timestamp_ms, 1609459200000);
    }

    #[test]
    fn test_parse_nan() {
        let ts = parse_line("metric nan 1000").unwrap();
        assert!(ts.samples[0].value.is_nan());
    }
}

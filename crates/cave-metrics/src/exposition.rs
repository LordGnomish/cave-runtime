// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus text exposition format parser.

#![allow(dead_code)]

use crate::error::{MetricsError, MetricsResult};
use crate::model::{Labels, MetricType, Timestamp, Value};

/// Parse the Prometheus text format.
/// Returns Vec<(Labels, Value, Option<Timestamp>)>.
pub fn parse_exposition(input: &str) -> MetricsResult<Vec<(Labels, Value, Option<Timestamp>)>> {
    let mut result = Vec::new();
    let mut _metric_types: std::collections::HashMap<String, MetricType> = std::collections::HashMap::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("# HELP ") {
            // Skip help lines
            continue;
        }
        if line.starts_with("# TYPE ") {
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            if parts.len() >= 4 {
                let metric_name = parts[2].to_string();
                let mt = match parts[3] {
                    "counter" => MetricType::Counter,
                    "gauge" => MetricType::Gauge,
                    "histogram" => MetricType::Histogram,
                    "summary" => MetricType::Summary,
                    _ => MetricType::Untyped,
                };
                _metric_types.insert(metric_name, mt);
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        // Parse: metric_name[{labels}] value [timestamp]
        match parse_sample_line(line) {
            Ok(Some(s)) => result.push(s),
            Ok(None) => {}
            Err(e) => tracing::warn!("Failed to parse exposition line: {}: {}", line, e),
        }
    }
    Ok(result)
}

fn parse_sample_line(line: &str) -> MetricsResult<Option<(Labels, Value, Option<Timestamp>)>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }

    // Find the metric name (up to '{' or whitespace)
    let (name_part, rest) = if let Some(idx) = line.find(|c: char| c == '{' || c.is_whitespace()) {
        (&line[..idx], line[idx..].trim_start_matches(|c: char| c.is_whitespace() && c != '{'))
    } else {
        return Err(MetricsError::Parse("invalid sample line".to_string()));
    };

    let metric_name = name_part.to_string();

    // Parse label set if present
    let (labels_map, rest2) = if rest.starts_with('{') {
        let close = rest.find('}').ok_or_else(|| MetricsError::Parse("unclosed {".to_string()))?;
        let label_str = &rest[1..close];
        let rest2 = rest[close + 1..].trim();
        let labels = parse_label_set(label_str)?;
        (labels, rest2)
    } else {
        (std::collections::BTreeMap::new(), rest)
    };

    // Build final labels including __name__
    let mut final_labels = labels_map;
    final_labels.insert("__name__".to_string(), metric_name);

    // Parse value and optional timestamp
    let mut parts = rest2.split_whitespace();
    let value_str = parts.next().ok_or_else(|| MetricsError::Parse("missing value".to_string()))?;
    let value: Value = match value_str {
        "+Inf" | "Inf" => f64::INFINITY,
        "-Inf" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        s => s.parse().map_err(|_| MetricsError::Parse(format!("bad value: {}", s)))?,
    };
    let timestamp: Option<Timestamp> = parts.next()
        .map(|s| s.parse::<i64>().map_err(|_| MetricsError::Parse(format!("bad timestamp: {}", s))))
        .transpose()?;

    Ok(Some((Labels(final_labels), value, timestamp)))
}

fn parse_label_set(s: &str) -> MetricsResult<std::collections::BTreeMap<String, String>> {
    let mut map = std::collections::BTreeMap::new();
    let s = s.trim();
    if s.is_empty() {
        return Ok(map);
    }
    let mut chars = s.char_indices().peekable();
    loop {
        // Skip whitespace
        while chars.peek().map(|(_, c)| c.is_whitespace()).unwrap_or(false) {
            chars.next();
        }
        if chars.peek().is_none() { break; }

        // Read key
        let key_start = chars.peek().map(|(i, _)| *i).unwrap_or(s.len());
        while chars.peek().map(|(_, c)| c.is_alphanumeric() || *c == '_').unwrap_or(false) {
            chars.next();
        }
        let key_end = chars.peek().map(|(i, _)| *i).unwrap_or(s.len());
        let key = s[key_start..key_end].to_string();
        if key.is_empty() { break; }

        // Skip whitespace and '='
        while chars.peek().map(|(_, c)| c.is_whitespace() || *c == '=').unwrap_or(false) {
            if chars.peek().map(|(_, c)| *c == '=').unwrap_or(false) {
                chars.next();
                break;
            }
            chars.next();
        }

        // Read quoted string value
        let quote = chars.peek().map(|(_, c)| *c);
        if quote != Some('"') && quote != Some('\'') {
            return Err(MetricsError::Parse(format!("expected quote in labels, got {:?}", quote)));
        }
        chars.next(); // consume quote
        let q = quote.unwrap();
        let mut value = String::new();
        loop {
            match chars.next() {
                None => return Err(MetricsError::Parse("unterminated string in labels".to_string())),
                Some((_, c)) if c == q => break,
                Some((_, '\\')) => {
                    match chars.next() {
                        Some((_, 'n')) => value.push('\n'),
                        Some((_, 't')) => value.push('\t'),
                        Some((_, '\\')) => value.push('\\'),
                        Some((_, c)) => value.push(c),
                        None => break,
                    }
                }
                Some((_, c)) => value.push(c),
            }
        }
        map.insert(key, value);

        // Skip comma
        while chars.peek().map(|(_, c)| c.is_whitespace()).unwrap_or(false) { chars.next(); }
        if chars.peek().map(|(_, c)| *c == ',').unwrap_or(false) { chars.next(); }
    }
    Ok(map)
}

//! Prometheus text exposition format parser (0.0.4) and OpenMetrics is handled
//! by openmetrics.rs.  This parser handles the standard `/metrics` text format.

use crate::error::{MetricsError, Result};
use crate::model::{Labels, MetricType, Sample, TimeSeries};
use super::IngestedBatch;

/// Parse a Prometheus text exposition body into a batch of time series.
pub fn parse(input: &str) -> Result<IngestedBatch> {
    let mut batch: std::collections::HashMap<u64, TimeSeries> = std::collections::HashMap::new();
    let mut _current_type = MetricType::Untyped;

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }

        if line.starts_with("# HELP ") {
            continue; // skip HELP comments
        }

        if line.starts_with("# TYPE ") {
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            if parts.len() >= 4 {
                _current_type = parts[3].parse().unwrap_or(MetricType::Untyped);
            }
            continue;
        }

        if line.starts_with('#') { continue; }

        match parse_line(line) {
            Ok((labels, value, opt_ts)) => {
                let fp = labels.fingerprint();
                let new_ts = TimeSeries {
                    labels: labels.clone(),
                    samples: vec![],
                };
                let entry = batch.entry(fp).or_insert(new_ts);
                let timestamp_ms = opt_ts.unwrap_or_else(now_ms);
                entry.samples.push(Sample::new(timestamp_ms, value));
            }
            Err(_) => continue,
        }
    }

    Ok(batch.into_values().collect())
}


fn parse_line(line: &str) -> Result<(Labels, f64, Option<i64>)> {
    // Format: metric_name[{label="value",...}] value [timestamp]
    let (name_and_labels, rest) = if let Some(idx) = line.find('{') {
        (&line[..idx], &line[idx..])
    } else {
        // No labels: split on whitespace
        let mut parts = line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim_start();
        let labels = Labels::from_pairs([("__name__", name)]);
        let (value, ts) = parse_value_and_ts(rest)?;
        return Ok((labels, value, ts));
    };

    let metric_name = name_and_labels.trim();

    // Parse label block
    let (label_str, remainder) = if rest.starts_with('{') {
        let end = rest.find('}').ok_or_else(|| MetricsError::Parse("unclosed {".into()))?;
        (&rest[1..end], rest[end+1..].trim())
    } else {
        ("", rest.trim())
    };

    let mut labels = parse_labels(label_str)?;
    if !metric_name.is_empty() {
        labels.insert("__name__", metric_name);
    }

    let (value, ts) = parse_value_and_ts(remainder)?;
    Ok((labels, value, ts))
}

fn parse_labels(s: &str) -> Result<Labels> {
    let mut labels = Labels::new();
    if s.is_empty() { return Ok(labels); }

    let mut rest = s.trim();
    while !rest.is_empty() {
        // name=
        let eq = rest.find('=').ok_or_else(|| MetricsError::Parse(format!("bad label pair: {}", rest)))?;
        let name = rest[..eq].trim().to_string();
        rest = rest[eq+1..].trim_start();

        // value: quoted string
        let (value, consumed) = if rest.starts_with('"') {
            parse_quoted_string(&rest[1..])?
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            (rest[..end].to_string(), end)
        };
        rest = rest[consumed..].trim_start();

        labels.insert(name, value);
        if rest.starts_with(',') { rest = rest[1..].trim_start(); }
    }
    Ok(labels)
}

fn parse_quoted_string(s: &str) -> Result<(String, usize)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    loop {
        match chars.next() {
            None => return Err(MetricsError::Parse("unclosed string".into())),
            Some((i, '"')) => return Ok((out, i + 2)), // +1 for opening quote, +1 for closing
            Some((_, '\\')) => match chars.next() {
                Some((_, 'n'))  => out.push('\n'),
                Some((_, 't'))  => out.push('\t'),
                Some((_, '\\'))=> out.push('\\'),
                Some((_, '"')) => out.push('"'),
                Some((_, c))   => { out.push('\\'); out.push(c); }
                None => return Err(MetricsError::Parse("bad escape".into())),
            },
            Some((_, c)) => out.push(c),
        }
    }
}

fn parse_value_and_ts(s: &str) -> Result<(f64, Option<i64>)> {
    let mut parts = s.split_whitespace();
    let val_str = parts.next().unwrap_or("NaN");
    let value = match val_str {
        "+Inf" | "Inf"  => f64::INFINITY,
        "-Inf"          => f64::NEG_INFINITY,
        "NaN"           => f64::NAN,
        v => v.parse::<f64>().map_err(|e| MetricsError::Parse(e.to_string()))?,
    };
    let ts = parts.next().and_then(|t| t.parse::<i64>().ok());
    Ok((value, ts))
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
    fn test_parse_simple_counter() {
        let input = r#"
# HELP http_requests_total Total HTTP requests
# TYPE http_requests_total counter
http_requests_total{method="GET",code="200"} 1234 1609459200000
"#;
        let batch = parse(input).unwrap();
        assert_eq!(batch.len(), 1);
        let ts = &batch[0];
        assert_eq!(ts.labels.get("__name__"), Some("http_requests_total"));
        assert_eq!(ts.labels.get("method"), Some("GET"));
        assert_eq!(ts.samples[0].value, 1234.0);
        assert_eq!(ts.samples[0].timestamp_ms, 1609459200000);
    }

    #[test]
    fn test_parse_no_labels() {
        let input = "go_goroutines 42\n";
        let batch = parse(input).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].samples[0].value, 42.0);
    }

    #[test]
    fn test_parse_inf_nan() {
        let input = "metric_inf +Inf\nmetric_nan NaN\n";
        let batch = parse(input).unwrap();
        assert_eq!(batch.len(), 2);
    }
}

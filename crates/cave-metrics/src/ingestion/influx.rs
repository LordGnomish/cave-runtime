//! InfluxDB line protocol parser.
//! Format: <measurement>[,tag_key=tag_val...] field_key=field_val[,...] [unix_timestamp_ns]
//! Reference: https://docs.influxdata.com/influxdb/v2/reference/syntax/line-protocol/

use crate::error::{MetricsError, Result};
use crate::model::{Labels, Sample, TimeSeries};
use super::IngestedBatch;

/// Parse an InfluxDB line protocol body into a batch of time series.
pub fn parse(input: &str) -> IngestedBatch {
    input.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .flat_map(|l| parse_line(l).into_iter())
        .collect()
}

/// Parse a single InfluxDB line protocol line.
/// Returns one TimeSeries per field.
pub fn parse_line(line: &str) -> Vec<TimeSeries> {
    match try_parse_line(line) {
        Ok(v) => v,
        Err(_) => vec![],
    }
}

fn try_parse_line(line: &str) -> Result<Vec<TimeSeries>> {
    // Split into: measurement+tags | fields | timestamp
    // The space is the delimiter but escaped spaces are allowed in tags.
    let (tags_part, rest) = split_on_unescaped_space(line)
        .ok_or_else(|| MetricsError::Parse("influx: missing fields".into()))?;

    let (fields_part, ts_part) = split_on_unescaped_space(rest)
        .map(|(f, t)| (f, Some(t)))
        .unwrap_or((rest, None));

    // Parse measurement and tags
    let (measurement, tags) = parse_measurement_tags(tags_part)?;

    // Parse timestamp (nanoseconds → milliseconds)
    let timestamp_ms = ts_part
        .and_then(|t| t.trim().parse::<i64>().ok())
        .map(|ns| ns / 1_000_000)
        .unwrap_or_else(now_ms);

    // Parse fields — each field becomes a separate time series
    let mut out = Vec::new();
    for field in parse_fields(fields_part) {
        let (field_key, field_value) = field;
        let mut labels = Labels::from_pairs(tags.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        // Metric name: measurement + "_" + field_key (Prometheus convention)
        let metric_name = if field_key == "value" {
            sanitize(&measurement)
        } else {
            format!("{}_{}", sanitize(&measurement), sanitize(&field_key))
        };
        labels.insert("__name__", metric_name);
        out.push(TimeSeries { labels, samples: vec![Sample::new(timestamp_ms, field_value)] });
    }
    Ok(out)
}

fn split_on_unescaped_space(s: &str) -> Option<(&str, &str)> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escaped = false;

    while i < chars.len() {
        if escaped { escaped = false; i += 1; continue; }
        if chars[i] == '\\' { escaped = true; i += 1; continue; }
        if chars[i] == '"' { in_string = !in_string; }
        if !in_string && chars[i] == ' ' {
            let (a, b) = s.split_at(i);
            return Some((a, b[1..].trim_start()));
        }
        i += 1;
    }
    None
}

fn parse_measurement_tags(s: &str) -> Result<(String, Vec<(String, String)>)> {
    let mut parts = s.splitn(2, ',');
    let measurement = unescape(parts.next().unwrap_or(""));
    let tag_str = parts.next().unwrap_or("");
    let tags = if tag_str.is_empty() {
        vec![]
    } else {
        tag_str.split(',').filter_map(|t| {
            let mut kv = t.splitn(2, '=');
            let k = kv.next().map(unescape)?;
            let v = kv.next().map(unescape)?;
            Some((k, v))
        }).collect()
    };
    Ok((measurement, tags))
}

fn parse_fields(s: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    // Naive split by comma outside of strings
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;

    for c in s.chars() {
        if escaped { current.push(c); escaped = false; continue; }
        if c == '\\' { escaped = true; current.push(c); continue; }
        if c == '"' { in_string = !in_string; current.push(c); continue; }
        if c == ',' && !in_string {
            if let Some(kv) = parse_field_kv(&current) { out.push(kv); }
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        if let Some(kv) = parse_field_kv(&current) { out.push(kv); }
    }
    out
}

fn parse_field_kv(s: &str) -> Option<(String, f64)> {
    let mut parts = s.splitn(2, '=');
    let key = parts.next()?.trim().to_string();
    let val_str = parts.next()?.trim();
    // Remove trailing 'i' (integer suffix), quotes (string), 't'/'f' (boolean)
    let val: f64 = if val_str.ends_with('i') {
        val_str[..val_str.len()-1].parse().ok()?
    } else if val_str.starts_with('"') {
        return None; // string fields not convertible
    } else if val_str == "t" || val_str == "T" || val_str.to_ascii_lowercase() == "true" {
        1.0
    } else if val_str == "f" || val_str == "F" || val_str.to_ascii_lowercase() == "false" {
        0.0
    } else {
        val_str.parse().ok()?
    };
    Some((key, val))
}

fn unescape(s: &str) -> String {
    s.replace("\\,", ",").replace("\\ ", " ").replace("\\=", "=")
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
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
        let ts = parse_line("cpu,host=web01 usage=0.85 1609459200000000000");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].labels.get("__name__"), Some("cpu_usage"));
        assert_eq!(ts[0].labels.get("host"), Some("web01"));
        assert!((ts[0].samples[0].value - 0.85).abs() < 1e-9);
        assert_eq!(ts[0].samples[0].timestamp_ms, 1609459200000);
    }

    #[test]
    fn test_multiple_fields() {
        let ts = parse_line("mem,host=web01 used=1024i,free=4096i 1000000000000");
        assert_eq!(ts.len(), 2);
    }

    #[test]
    fn test_no_tags() {
        let ts = parse_line("temperature value=21.5 1609459200000000000");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].labels.get("__name__"), Some("temperature"));
        assert!((ts[0].samples[0].value - 21.5).abs() < 1e-9);
    }
}

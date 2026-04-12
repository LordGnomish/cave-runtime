//! Scrape targets: Prometheus exposition format parsing, service discovery.

use crate::models::{Sample, ScrapeTarget};
use crate::storage::{insert_samples, TimeSeriesStore};
use chrono::Utc;
use std::collections::HashMap;

/// Parsed metric line from Prometheus exposition format.
#[derive(Debug, Clone)]
pub struct ParsedMetric {
    pub name: String,
    pub labels: HashMap<String, String>,
    pub value: f64,
    pub timestamp_ms: Option<i64>,
}

/// Parse a Prometheus text exposition format body.
/// Handles `# HELP`, `# TYPE`, metric lines with optional labels and timestamp.
pub fn parse_prometheus_exposition(body: &str) -> Vec<ParsedMetric> {
    let mut out = Vec::new();

    for line in body.lines() {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // metric_name{labels} value [timestamp_ms]
        // or metric_name value [timestamp_ms]
        let (name_and_labels, rest) = if let Some(brace) = line.find('{') {
            let close = line.find('}').unwrap_or(line.len());
            let name = &line[..brace];
            let label_str = &line[brace + 1..close];
            let after = line[close + 1..].trim();
            (
                (name.to_string(), parse_label_set(label_str)),
                after,
            )
        } else {
            let mut parts = line.splitn(2, ' ');
            let name = parts.next().unwrap_or("").to_string();
            let after = parts.next().unwrap_or("").trim();
            ((name, HashMap::new()), after)
        };

        let mut tokens = rest.split_whitespace();
        let value: f64 = match tokens.next().and_then(|v| v.parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        let timestamp_ms: Option<i64> = tokens.next().and_then(|t| t.parse().ok());

        out.push(ParsedMetric {
            name: name_and_labels.0,
            labels: name_and_labels.1,
            value,
            timestamp_ms,
        });
    }

    out
}

fn parse_label_set(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in s.split(',') {
        let part = part.trim();
        if let Some(eq) = part.find('=') {
            let key = part[..eq].trim().to_string();
            let val = part[eq + 1..].trim().trim_matches('"').to_string();
            if !key.is_empty() {
                out.insert(key, val);
            }
        }
    }
    out
}

/// Scrape all enabled targets, parse exposition, insert into store.
pub async fn scrape_targets(
    targets: &mut Vec<ScrapeTarget>,
    store: &mut TimeSeriesStore,
) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    for target in targets.iter_mut() {
        if !target.enabled {
            continue;
        }
        let url = target.url();
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.text().await {
                    Ok(body) => {
                        let metrics = parse_prometheus_exposition(&body);
                        let now = Utc::now();
                        for m in metrics {
                            let ts = m.timestamp_ms.map(|ms| {
                                chrono::DateTime::from_timestamp_millis(ms)
                                    .unwrap_or(now)
                            }).unwrap_or(now);

                            let mut labels = m.labels.clone();
                            // Add target labels
                            labels.insert("job".to_string(), target.job.clone());
                            labels.insert("instance".to_string(), target.address.clone());
                            for (k, v) in &target.labels {
                                labels.entry(k.clone()).or_insert_with(|| v.clone());
                            }

                            insert_samples(
                                store,
                                &m.name,
                                &labels,
                                vec![Sample { timestamp: ts, value: m.value }],
                            );
                        }
                        target.last_scrape = Some(Utc::now());
                        target.last_error = None;
                    }
                    Err(e) => {
                        target.last_error = Some(format!("body error: {e}"));
                    }
                }
            }
            Ok(resp) => {
                target.last_error = Some(format!("HTTP {}", resp.status()));
            }
            Err(e) => {
                target.last_error = Some(e.to_string());
            }
        }
    }
}

/// Static service discovery: return targets derived from a list of addresses for a job.
pub fn service_discovery(job: &str, addresses: &[&str]) -> Vec<ScrapeTarget> {
    addresses.iter().map(|addr| ScrapeTarget::new(job, *addr)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_metric() {
        let body = "http_requests_total 1027\n";
        let metrics = parse_prometheus_exposition(body);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].name, "http_requests_total");
        assert!((metrics[0].value - 1027.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_with_labels() {
        let body = r#"http_requests_total{method="GET",status="200"} 42"#;
        let metrics = parse_prometheus_exposition(body);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].labels.get("method").map(|s| s.as_str()), Some("GET"));
        assert!((metrics[0].value - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_skips_comments() {
        let body = "# HELP foo A counter\n# TYPE foo counter\nfoo 1\n";
        let metrics = parse_prometheus_exposition(body);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].name, "foo");
    }

    #[test]
    fn test_parse_with_timestamp() {
        let body = "cpu_usage 0.75 1712345678000\n";
        let metrics = parse_prometheus_exposition(body);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].timestamp_ms, Some(1712345678000));
    }

    #[test]
    fn test_service_discovery() {
        let targets = service_discovery("api", &["localhost:9090", "localhost:9091"]);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].job, "api");
    }
}

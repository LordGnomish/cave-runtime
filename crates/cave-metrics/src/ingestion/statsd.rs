//! StatsD protocol parser.
//! Format: metric.name:value|type[|@rate][|#tag1:val1,tag2:val2]
//! Types: c (counter), g (gauge), ms (timer/histogram), s (set), h (histogram)

use crate::error::{MetricsError, Result};
use crate::model::{Labels, Sample, TimeSeries};
use super::IngestedBatch;

#[derive(Debug)]
pub struct StatsdPacket {
    pub name: String,
    pub value: f64,
    pub metric_type: StatsdType,
    pub sample_rate: f64,
    pub tags: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatsdType {
    Counter,
    Gauge,
    Timer,
    Set,
    Histogram,
}

/// Parse a single StatsD UDP packet line.
pub fn parse_packet(line: &str) -> Result<StatsdPacket> {
    // Split on | to get components
    let mut parts = line.splitn(2, ':');
    let name = parts.next().ok_or_else(|| MetricsError::Parse("empty statsd packet".into()))?.to_string();
    let rest = parts.next().ok_or_else(|| MetricsError::Parse("missing value in statsd packet".into()))?;

    let mut segments = rest.split('|');
    let value_str = segments.next().unwrap_or("0");
    let value: f64 = value_str.parse().map_err(|e| MetricsError::Parse(format!("statsd value: {}", e)))?;

    let type_str = segments.next().unwrap_or("g");
    let metric_type = match type_str {
        "c"  => StatsdType::Counter,
        "g"  => StatsdType::Gauge,
        "ms" => StatsdType::Timer,
        "s"  => StatsdType::Set,
        "h"  => StatsdType::Histogram,
        _    => StatsdType::Gauge,
    };

    let mut sample_rate = 1.0f64;
    let mut tags = Vec::new();

    for segment in segments {
        if let Some(rate_str) = segment.strip_prefix('@') {
            sample_rate = rate_str.parse().unwrap_or(1.0);
        } else if let Some(tag_str) = segment.strip_prefix('#') {
            // DogStatsD tags: tag1:val1,tag2:val2
            for tag in tag_str.split(',') {
                let mut kv = tag.splitn(2, ':');
                let k = kv.next().unwrap_or("").to_string();
                let v = kv.next().unwrap_or("true").to_string();
                if !k.is_empty() { tags.push((k, v)); }
            }
        }
    }

    Ok(StatsdPacket { name, value, metric_type, sample_rate, tags })
}

/// Convert a StatsD packet to a TimeSeries.
pub fn packet_to_timeseries(packet: StatsdPacket) -> TimeSeries {
    let ts_ms = now_ms();
    let mut labels = Labels::new();
    labels.insert("__name__", sanitize_name(&packet.name));
    for (k, v) in packet.tags {
        labels.insert(k, v);
    }

    // Apply sample rate for counters
    let value = if packet.metric_type == StatsdType::Counter && packet.sample_rate > 0.0 {
        packet.value / packet.sample_rate
    } else {
        packet.value
    };

    TimeSeries { labels, samples: vec![Sample::new(ts_ms, value)] }
}

/// Parse a multi-line StatsD batch (newline-separated packets).
pub fn parse_batch(input: &str) -> IngestedBatch {
    input.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| parse_packet(l).ok())
        .map(packet_to_timeseries)
        .collect()
}

fn sanitize_name(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
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
    fn test_parse_counter() {
        let pkt = parse_packet("requests:1|c|@0.1|#method:GET,code:200").unwrap();
        assert_eq!(pkt.name, "requests");
        assert_eq!(pkt.value, 1.0);
        assert_eq!(pkt.metric_type, StatsdType::Counter);
        assert!((pkt.sample_rate - 0.1).abs() < 1e-9);
        assert_eq!(pkt.tags[0], ("method".to_string(), "GET".to_string()));
    }

    #[test]
    fn test_parse_gauge() {
        let pkt = parse_packet("memory.usage:1024|g").unwrap();
        assert_eq!(pkt.metric_type, StatsdType::Gauge);
        assert_eq!(pkt.value, 1024.0);
    }
}

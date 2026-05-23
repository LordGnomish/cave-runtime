// SPDX-License-Identifier: AGPL-3.0-or-later
//! Access log line writer — JSON, single-line per request.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogEntry {
    pub ts: String, pub method: String, pub uri: String, pub host: String,
    pub status: u16, pub bytes_sent: u64, pub duration_ms: u64,
    pub route: Option<String>, pub service: Option<String>, pub upstream_host: Option<String>,
    pub consumer: Option<String>, pub source_ip: Option<String>, pub user_agent: Option<String>,
    pub plugin_chain: Vec<String>,
}

impl AccessLogEntry {
    pub fn json_line(&self) -> Result<String, serde_json::Error> { serde_json::to_string(self) }
}

#[derive(Default)]
pub struct AccessLogBuilder {
    pub method: String, pub uri: String, pub host: String,
    pub status: u16, pub bytes_sent: u64, pub elapsed: Duration,
    pub route: Option<String>, pub service: Option<String>,
    pub upstream_host: Option<String>, pub consumer: Option<String>,
    pub source_ip: Option<String>, pub user_agent: Option<String>,
    pub plugin_chain: Vec<String>,
}
impl AccessLogBuilder {
    pub fn new() -> Self { Self::default() }
    pub fn build(self) -> AccessLogEntry {
        AccessLogEntry {
            ts: chrono::Utc::now().to_rfc3339(),
            method: self.method, uri: self.uri, host: self.host,
            status: self.status, bytes_sent: self.bytes_sent,
            duration_ms: self.elapsed.as_millis() as u64,
            route: self.route, service: self.service,
            upstream_host: self.upstream_host, consumer: self.consumer,
            source_ip: self.source_ip, user_agent: self.user_agent,
            plugin_chain: self.plugin_chain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn entry_serializes() {
        let e = AccessLogBuilder { method: "GET".into(), uri: "/".into(), host: "h".into(), status: 200, ..Default::default() }.build();
        let j = e.json_line().unwrap();
        assert!(j.contains("\"method\":\"GET\""));
    }
    #[test] fn duration_in_ms() {
        let e = AccessLogBuilder { elapsed: Duration::from_millis(125), status: 200, ..Default::default() }.build();
        assert_eq!(e.duration_ms, 125);
    }
    #[test] fn plugin_chain_recorded() {
        let e = AccessLogBuilder { plugin_chain: vec!["a".into(), "b".into()], status: 200, ..Default::default() }.build();
        assert_eq!(e.plugin_chain, vec!["a", "b"]);
    }
    #[test] fn timestamp_present() {
        let e = AccessLogBuilder { status: 200, ..Default::default() }.build();
        assert!(!e.ts.is_empty());
    }
    #[test] fn json_roundtrip() {
        let e = AccessLogBuilder { method: "POST".into(), status: 201, ..Default::default() }.build();
        let j = e.json_line().unwrap();
        let e2: AccessLogEntry = serde_json::from_str(&j).unwrap();
        assert_eq!(e2.method, "POST");
    }
}

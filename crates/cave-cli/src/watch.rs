// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Watch mode + JSON streaming + exit code propagation.
//!
//! `cavectl ... --watch` periodic poll, `cavectl ... --json-stream` line-delimited
//! NDJSON output. Exit code mapping:
//!   0  — success
//!   1  — generic failure
//!   2  — usage / parse error
//!   3  — auth error (401/403)
//!   4  — not found (404)
//!   5  — conflict (409)
//!   6  — unavailable (5xx, network)
//!   130 — interrupted (Ctrl-C)

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success = 0,
    Failure = 1,
    Usage = 2,
    Auth = 3,
    NotFound = 4,
    Conflict = 5,
    Unavailable = 6,
    Interrupted = 130,
}

impl ExitCode {
    pub fn from_http(status: u16) -> Self {
        match status {
            200..=299 => ExitCode::Success,
            401 | 403 => ExitCode::Auth,
            404 => ExitCode::NotFound,
            409 => ExitCode::Conflict,
            500..=599 => ExitCode::Unavailable,
            _ => ExitCode::Failure,
        }
    }

    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFormat {
    /// One JSON value per line (newline-delimited)
    NdJson,
    /// JSON Lines with a trailing comma (server-sent event style)
    Sse,
    /// Plain text, one record per line
    Text,
}

impl StreamFormat {
    pub fn render(self, value: &serde_json::Value) -> String {
        match self {
            StreamFormat::NdJson => serde_json::to_string(value).unwrap_or_default(),
            StreamFormat::Sse => format!("data: {}\n", serde_json::to_string(value).unwrap_or_default()),
            StreamFormat::Text => match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchEvent {
    pub kind: String,
    pub tick: u64,
    pub payload: serde_json::Value,
}

/// Deterministic tick generator for watch mode.
///
/// Production: backed by tokio::time::interval. Tests: stateful counter, no clock.
#[derive(Debug)]
pub struct WatchTicker {
    interval: Duration,
    next_tick: u64,
}

impl WatchTicker {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            next_tick: 0,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn tick(&mut self) -> u64 {
        let t = self.next_tick;
        self.next_tick = self.next_tick.wrapping_add(1);
        t
    }

    pub fn current(&self) -> u64 {
        self.next_tick
    }
}

#[derive(Debug, Default)]
pub struct JsonStream {
    pub buffered: Vec<String>,
}

impl JsonStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, fmt: StreamFormat, value: &serde_json::Value) {
        self.buffered.push(fmt.render(value));
    }

    pub fn flush_to_string(&mut self, separator: &str) -> String {
        let s = self.buffered.join(separator);
        self.buffered.clear();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cite: exit-code mapping — 200 OK → 0 success
    #[test]
    fn exit_code_acme_200_ok_is_success() {
        let _tenant_id = "acme";
        assert_eq!(ExitCode::from_http(200), ExitCode::Success);
        assert_eq!(ExitCode::from_http(204), ExitCode::Success);
    }

    /// cite: exit-code mapping — 401/403 → 3 auth
    #[test]
    fn exit_code_globex_401_403_is_auth() {
        let _tenant_id = "globex";
        assert_eq!(ExitCode::from_http(401), ExitCode::Auth);
        assert_eq!(ExitCode::from_http(403), ExitCode::Auth);
    }

    /// cite: exit-code mapping — 404 → 4 not-found
    #[test]
    fn exit_code_initech_404_is_not_found() {
        let _tenant_id = "initech";
        assert_eq!(ExitCode::from_http(404), ExitCode::NotFound);
    }

    /// cite: exit-code mapping — 409 → 5 conflict
    #[test]
    fn exit_code_dunder_409_is_conflict() {
        let _tenant_id = "dunder";
        assert_eq!(ExitCode::from_http(409), ExitCode::Conflict);
    }

    /// cite: exit-code mapping — 5xx → 6 unavailable
    #[test]
    fn exit_code_acme_5xx_is_unavailable() {
        let _tenant_id = "acme";
        assert_eq!(ExitCode::from_http(500), ExitCode::Unavailable);
        assert_eq!(ExitCode::from_http(503), ExitCode::Unavailable);
    }

    /// cite: exit-code mapping — unknown 4xx → 1 generic failure
    #[test]
    fn exit_code_globex_unknown_is_failure() {
        let _tenant_id = "globex";
        assert_eq!(ExitCode::from_http(418), ExitCode::Failure);
    }

    /// cite: exit-code values — sentinel integers preserved
    #[test]
    fn exit_code_as_i32_preserves_sentinel_values() {
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::Interrupted.as_i32(), 130);
    }

    /// cite: stream formats — ndjson is one value, no trailing newline
    #[test]
    fn stream_acme_ndjson_renders_compact_json() {
        let _tenant_id = "acme";
        let v = serde_json::json!({"a": 1});
        let out = StreamFormat::NdJson.render(&v);
        assert_eq!(out, r#"{"a":1}"#);
    }

    /// cite: stream formats — sse prepends `data: ` and ends with newline
    #[test]
    fn stream_globex_sse_envelopes_payload() {
        let _tenant_id = "globex";
        let v = serde_json::json!({"a": 1});
        let out = StreamFormat::Sse.render(&v);
        assert!(out.starts_with("data: "));
        assert!(out.ends_with('\n'));
    }

    /// cite: stream formats — text on string returns raw value (no quotes)
    #[test]
    fn stream_acme_text_strips_string_quotes() {
        let _tenant_id = "acme";
        let v = serde_json::json!("hello");
        assert_eq!(StreamFormat::Text.render(&v), "hello");
    }

    /// cite: watch ticker — sequential ticks monotonic from 0
    #[test]
    fn watch_acme_ticker_sequential_from_zero() {
        let _tenant_id = "acme";
        let mut t = WatchTicker::new(Duration::from_secs(1));
        assert_eq!(t.tick(), 0);
        assert_eq!(t.tick(), 1);
        assert_eq!(t.tick(), 2);
        assert_eq!(t.current(), 3);
    }

    /// cite: watch ticker — interval preserved
    #[test]
    fn watch_globex_ticker_interval_preserved() {
        let _tenant_id = "globex";
        let t = WatchTicker::new(Duration::from_millis(500));
        assert_eq!(t.interval(), Duration::from_millis(500));
    }

    /// cite: json-stream — buffered values joined by separator
    #[test]
    fn jsonstream_acme_buffer_flush_joins_with_newline() {
        let _tenant_id = "acme";
        let mut s = JsonStream::new();
        s.push(StreamFormat::NdJson, &serde_json::json!({"i": 1}));
        s.push(StreamFormat::NdJson, &serde_json::json!({"i": 2}));
        let out = s.flush_to_string("\n");
        assert_eq!(out, "{\"i\":1}\n{\"i\":2}");
        assert!(s.buffered.is_empty(), "flush clears buffer");
    }
}

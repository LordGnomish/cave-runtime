// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for HTTP / TCP / DNS / Push probe execution and helpers.

use cave_uptime::probe::{
    HttpProbeConfig, ProbeError, build_probe_result, evaluate_push_probe,
    execute_dns_probe, execute_http_probe, execute_tcp_probe,
};
use chrono::Utc;
use uuid::Uuid;

// ─── HttpProbeConfig construction ────────────────────────────────────────────

#[test]
fn test_http_probe_config_defaults() {
    let cfg = HttpProbeConfig::new("https://example.com".to_string());
    assert_eq!(cfg.method, "GET");
    assert_eq!(cfg.timeout_ms, 5000);
    assert!(cfg.follow_redirects);
    assert!(cfg.keyword.is_none());
    assert!(cfg.expected_status.is_none());
}

#[test]
fn test_http_probe_config_keyword_set() {
    let cfg = HttpProbeConfig {
        url: "https://example.com".to_string(),
        timeout_ms: 3000,
        method: "GET".to_string(),
        keyword: Some("OK".to_string()),
        expected_status: Some(200),
        follow_redirects: true,
    };
    assert_eq!(cfg.keyword, Some("OK".to_string()));
    assert_eq!(cfg.expected_status, Some(200));
}

// ─── ProbeError display ───────────────────────────────────────────────────────

#[test]
fn test_probe_error_timeout_display() {
    let e = ProbeError::Timeout("5000ms".to_string());
    let s = e.to_string();
    assert!(!s.is_empty());
}

#[test]
fn test_probe_error_connection_display() {
    let e = ProbeError::ConnectionFailed("refused".to_string());
    let s = e.to_string();
    assert!(!s.is_empty());
}

// ─── build_probe_result helper ────────────────────────────────────────────────

#[test]
fn test_build_probe_result_success() {
    let id = Uuid::new_v4();
    let r = build_probe_result(id, true, 42, Some(200), None);
    assert_eq!(r.probe_id, id);
    assert!(r.success);
    assert_eq!(r.latency_ms, 42);
    assert_eq!(r.status_code, Some(200));
    assert!(r.error.is_none());
}

#[test]
fn test_build_probe_result_failure() {
    let id = Uuid::new_v4();
    let r = build_probe_result(id, false, 0, None, Some("timeout".to_string()));
    assert!(!r.success);
    assert_eq!(r.error, Some("timeout".to_string()));
}

// ─── Push probe evaluation ────────────────────────────────────────────────────

#[test]
fn test_push_probe_no_push_yet() {
    let id = Uuid::new_v4();
    let r = evaluate_push_probe(id, None, 60);
    assert!(!r.success);
    assert!(r.error.is_some());
}

#[test]
fn test_push_probe_fresh_heartbeat() {
    let id = Uuid::new_v4();
    let now = Utc::now().timestamp();
    let r = evaluate_push_probe(id, Some(now - 5), 60);
    assert!(r.success, "push 5s ago with 60s interval should be UP");
}

#[test]
fn test_push_probe_overdue() {
    let id = Uuid::new_v4();
    let now = Utc::now().timestamp();
    // Last push 200s ago, interval 60s — overdue
    let r = evaluate_push_probe(id, Some(now - 200), 60);
    assert!(!r.success);
    assert!(r.error.as_deref().unwrap_or("").contains("overdue"));
}

#[test]
fn test_push_probe_within_grace() {
    let id = Uuid::new_v4();
    let now = Utc::now().timestamp();
    // 80s ago with 60s interval → within 30s grace
    let r = evaluate_push_probe(id, Some(now - 80), 60);
    assert!(r.success, "should be within grace period");
}

// ─── TCP probe ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_tcp_probe_invalid_host() {
    let id = Uuid::new_v4();
    let r = execute_tcp_probe(id, "this.invalid.hostname.xyzabc123", 80, 2000).await;
    assert_eq!(r.probe_id, id);
    assert!(!r.success);
    assert!(r.error.is_some());
}

#[tokio::test]
async fn test_tcp_probe_timeout_short() {
    // 192.0.2.1 is TEST-NET-1 (RFC 5737) — packets are dropped → triggers timeout
    let id = Uuid::new_v4();
    let r = execute_tcp_probe(id, "192.0.2.1", 9999, 200).await;
    assert_eq!(r.probe_id, id);
    assert!(!r.success);
}

#[tokio::test]
async fn test_tcp_probe_result_has_id() {
    let id = Uuid::new_v4();
    let r = execute_tcp_probe(id, "127.0.0.1", 19998, 500).await;
    assert_eq!(r.probe_id, id);
}

// ─── DNS probe ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_dns_probe_localhost_resolves() {
    let id = Uuid::new_v4();
    let r = execute_dns_probe(id, "localhost", 5000).await;
    assert_eq!(r.probe_id, id);
    assert!(r.success, "localhost should always resolve");
}

#[tokio::test]
async fn test_dns_probe_invalid_domain() {
    let id = Uuid::new_v4();
    let r = execute_dns_probe(id, "nxdomain.invalid.xyzabc123456", 3000).await;
    assert!(!r.success);
    assert!(r.error.is_some());
}

// ─── HTTP probe async ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_http_probe_invalid_url() {
    let id = Uuid::new_v4();
    let cfg = HttpProbeConfig::new("http://this.host.does.not.exist.xyzabc123:9999/".to_string());
    let r = execute_http_probe(id, &cfg).await;
    assert_eq!(r.probe_id, id);
    assert!(!r.success);
    assert!(r.error.is_some());
}

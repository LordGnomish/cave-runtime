// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for status page model and retry logic.

use cave_uptime::status::{
    MonitorStatus, ProbeStatusSummary, StatusPage, StatusPageEntry, build_status_page,
};
use cave_uptime::retry::{RetryConfig, RetryResult, execute_with_retry};
use cave_uptime::models::{ProbeType, UptimeProbe};
use uuid::Uuid;

// ─── MonitorStatus ────────────────────────────────────────────────────────────

#[test]
fn test_monitor_status_variants() {
    let up = MonitorStatus::Up;
    let down = MonitorStatus::Down;
    let pending = MonitorStatus::Pending;
    let maintenance = MonitorStatus::Maintenance;
    assert_eq!(up.label(), "up");
    assert_eq!(down.label(), "down");
    assert_eq!(pending.label(), "pending");
    assert_eq!(maintenance.label(), "maintenance");
}

#[test]
fn test_monitor_status_is_up() {
    assert!(MonitorStatus::Up.is_up());
    assert!(!MonitorStatus::Down.is_up());
    assert!(!MonitorStatus::Pending.is_up());
}

// ─── ProbeStatusSummary ───────────────────────────────────────────────────────

#[test]
fn test_probe_status_summary_construction() {
    let id = Uuid::new_v4();
    let s = ProbeStatusSummary {
        probe_id: id,
        name: "API Health".to_string(),
        status: MonitorStatus::Up,
        uptime_24h: 99.5,
        avg_latency_ms: 42.0,
        last_check_ms: 35,
    };
    assert_eq!(s.probe_id, id);
    assert!(s.status.is_up());
    assert!((s.uptime_24h - 99.5).abs() < 0.01);
}

// ─── StatusPage ───────────────────────────────────────────────────────────────

#[test]
fn test_status_page_all_up() {
    let probes = vec![
        make_probe("API", ProbeType::Http),
        make_probe("DB", ProbeType::Tcp),
    ];
    let summaries: Vec<ProbeStatusSummary> = probes
        .iter()
        .map(|p| ProbeStatusSummary {
            probe_id: p.id,
            name: p.name.clone(),
            status: MonitorStatus::Up,
            uptime_24h: 100.0,
            avg_latency_ms: 50.0,
            last_check_ms: 45,
        })
        .collect();
    let page = build_status_page("My Platform", summaries);
    assert_eq!(page.title, "My Platform");
    assert!(page.all_operational());
    assert_eq!(page.entries.len(), 2);
}

#[test]
fn test_status_page_partial_outage() {
    let summaries = vec![
        ProbeStatusSummary {
            probe_id: Uuid::new_v4(),
            name: "API".to_string(),
            status: MonitorStatus::Up,
            uptime_24h: 100.0,
            avg_latency_ms: 50.0,
            last_check_ms: 45,
        },
        ProbeStatusSummary {
            probe_id: Uuid::new_v4(),
            name: "DB".to_string(),
            status: MonitorStatus::Down,
            uptime_24h: 90.0,
            avg_latency_ms: 0.0,
            last_check_ms: 0,
        },
    ];
    let page = build_status_page("My Platform", summaries);
    assert!(!page.all_operational());
    assert_eq!(page.down_count(), 1);
    assert_eq!(page.up_count(), 1);
}

#[test]
fn test_status_page_empty() {
    let page = build_status_page("Empty", vec![]);
    assert!(page.all_operational());
    assert_eq!(page.entries.len(), 0);
}

// ─── RetryConfig ─────────────────────────────────────────────────────────────

#[test]
fn test_retry_config_defaults() {
    let cfg = RetryConfig::default();
    assert!(cfg.max_attempts > 0);
    assert!(cfg.base_delay_ms > 0);
}

#[test]
fn test_retry_config_custom() {
    let cfg = RetryConfig {
        max_attempts: 3,
        base_delay_ms: 200,
        max_delay_ms: 5000,
        backoff_multiplier: 2.0,
    };
    assert_eq!(cfg.max_attempts, 3);
    let delay1 = cfg.delay_for_attempt(0);
    let delay2 = cfg.delay_for_attempt(1);
    assert!(delay2 > delay1, "delay should increase with attempt");
    let delay_max = cfg.delay_for_attempt(100);
    assert!(delay_max <= cfg.max_delay_ms, "delay capped at max_delay_ms");
}

#[tokio::test]
async fn test_execute_with_retry_immediate_success() {
    let cfg = RetryConfig::default();
    let mut call_count = 0usize;
    let result = execute_with_retry(&cfg, || {
        call_count += 1;
        async { Ok::<u32, String>(42) }
    }).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
    assert_eq!(call_count, 1, "should succeed on first attempt");
}

#[tokio::test]
async fn test_execute_with_retry_eventual_success() {
    let cfg = RetryConfig {
        max_attempts: 3,
        base_delay_ms: 1, // minimal delay for fast tests
        max_delay_ms: 10,
        backoff_multiplier: 1.0,
    };
    let mut call_count = 0usize;
    let result = execute_with_retry(&cfg, || {
        call_count += 1;
        let count = call_count;
        async move {
            if count < 3 {
                Err("not yet".to_string())
            } else {
                Ok::<u32, String>(99)
            }
        }
    }).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 99);
    assert_eq!(call_count, 3);
}

#[tokio::test]
async fn test_execute_with_retry_all_fail() {
    let cfg = RetryConfig {
        max_attempts: 2,
        base_delay_ms: 1,
        max_delay_ms: 10,
        backoff_multiplier: 1.0,
    };
    let result = execute_with_retry(&cfg, || {
        async { Err::<u32, String>("always fails".to_string()) }
    }).await;
    assert!(result.is_err());
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn make_probe(name: &str, probe_type: ProbeType) -> UptimeProbe {
    UptimeProbe {
        id: Uuid::new_v4(),
        name: name.to_string(),
        target_url: "http://example.com".to_string(),
        probe_type,
        interval_seconds: 60,
        timeout_ms: 5000,
        enabled: true,
    }
}

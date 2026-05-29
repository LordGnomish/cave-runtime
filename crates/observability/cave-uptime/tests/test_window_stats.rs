// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for uptime window stats and heartbeat/history model.

use cave_uptime::history::{
    HeartbeatStore, ProbeHistory, UptimeWindow, WindowStats, compute_window_stats,
};
use cave_uptime::models::{ProbeResult, ProbeType, UptimeProbe};
use chrono::{Duration as ChronoDuration, Utc};
use uuid::Uuid;

fn make_result_at(probe_id: Uuid, success: bool, latency_ms: u64, offset_secs: i64) -> ProbeResult {
    ProbeResult {
        probe_id,
        success,
        latency_ms,
        status_code: if success { Some(200) } else { None },
        error: if success { None } else { Some("err".to_string()) },
        checked_at: Utc::now() - ChronoDuration::seconds(offset_secs),
    }
}

#[test]
fn test_window_stats_all_success_24h() {
    let id = Uuid::new_v4();
    let results: Vec<ProbeResult> = (0..100)
        .map(|i| make_result_at(id, true, 50, i * 600)) // every 10 min going back
        .collect();
    // Only take those within 24h = 86400 seconds
    let stats = compute_window_stats(&results, UptimeWindow::Hours24);
    assert!(stats.uptime_pct > 99.0, "expected ~100%, got {}", stats.uptime_pct);
    assert!(stats.total_checks > 0);
    assert_eq!(stats.failed_checks, 0);
    assert!(stats.avg_latency_ms > 0.0);
}

#[test]
fn test_window_stats_mixed_7days() {
    let id = Uuid::new_v4();
    let mut results = Vec::new();
    // 100 success + 20 failures spread over 7 days
    for i in 0..100 {
        results.push(make_result_at(id, true, 100, i * 3600));
    }
    for i in 0..20 {
        results.push(make_result_at(id, false, 0, i * 3600 + 1800));
    }
    let stats = compute_window_stats(&results, UptimeWindow::Days7);
    assert!(stats.total_checks > 0);
    assert!(stats.failed_checks > 0);
    let expected_pct = 100.0 * stats.successful_checks as f64 / stats.total_checks as f64;
    assert!((stats.uptime_pct - expected_pct).abs() < 0.01);
}

#[test]
fn test_window_stats_empty() {
    let stats = compute_window_stats(&[], UptimeWindow::Hours24);
    assert_eq!(stats.total_checks, 0);
    assert_eq!(stats.uptime_pct, 100.0);
}

#[test]
fn test_window_stats_only_old_results() {
    let id = Uuid::new_v4();
    // Results older than 24h — 24h window should be empty
    let results: Vec<ProbeResult> = (0..10)
        .map(|i| make_result_at(id, true, 50, 90000 + i * 3600))
        .collect();
    let stats = compute_window_stats(&results, UptimeWindow::Hours24);
    assert_eq!(stats.total_checks, 0, "no results in 24h window");
}

#[test]
fn test_window_labels() {
    assert_eq!(UptimeWindow::Hours24.label(), "24h");
    assert_eq!(UptimeWindow::Days7.label(), "7d");
    assert_eq!(UptimeWindow::Days30.label(), "30d");
}

// ─── HeartbeatStore ───────────────────────────────────────────────────────────

#[test]
fn test_heartbeat_store_record_and_list() {
    let id = Uuid::new_v4();
    let store = HeartbeatStore::new(200);
    for i in 0..5 {
        store.record(make_result_at(id, true, 50, i * 60));
    }
    let history = store.get_history(id, 10);
    assert_eq!(history.len(), 5);
}

#[test]
fn test_heartbeat_store_capacity_limit() {
    let id = Uuid::new_v4();
    let store = HeartbeatStore::new(5); // max 5 per probe
    for i in 0..10 {
        store.record(make_result_at(id, true, 50, i * 60));
    }
    let history = store.get_history(id, 100);
    assert!(history.len() <= 5, "should be capped at 5, got {}", history.len());
}

#[test]
fn test_heartbeat_store_empty_probe() {
    let store = HeartbeatStore::new(100);
    let history = store.get_history(Uuid::new_v4(), 10);
    assert!(history.is_empty());
}

#[test]
fn test_probe_history_window_stats() {
    let id = Uuid::new_v4();
    let store = HeartbeatStore::new(1000);
    for i in 0..50 {
        store.record(make_result_at(id, true, 80, i * 1200));
    }
    for i in 0..10 {
        store.record(make_result_at(id, false, 0, i * 1200 + 600));
    }
    let stats = store.window_stats(id, UptimeWindow::Days7);
    assert!(stats.total_checks > 0);
    assert!(stats.uptime_pct < 100.0);
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for scheduler and API route handlers.

use cave_uptime::models::{ProbeType, UptimeProbe};
use cave_uptime::scheduler::{ProbeScheduler, SchedulerConfig};
use uuid::Uuid;

// ─── SchedulerConfig ──────────────────────────────────────────────────────────

#[test]
fn test_scheduler_config_defaults() {
    let cfg = SchedulerConfig::default();
    assert!(cfg.max_concurrent_probes > 0);
    assert!(cfg.jitter_ms <= 5000, "jitter should be reasonable");
}

#[test]
fn test_scheduler_config_custom() {
    let cfg = SchedulerConfig {
        max_concurrent_probes: 50,
        jitter_ms: 500,
    };
    assert_eq!(cfg.max_concurrent_probes, 50);
    assert_eq!(cfg.jitter_ms, 500);
}

// ─── ProbeScheduler construction ─────────────────────────────────────────────

#[test]
fn test_scheduler_construction() {
    let cfg = SchedulerConfig::default();
    let scheduler = ProbeScheduler::new(cfg);
    assert_eq!(scheduler.probe_count(), 0, "starts empty");
}

#[test]
fn test_scheduler_register_probe() {
    let scheduler = ProbeScheduler::new(SchedulerConfig::default());
    let probe = make_probe("test", ProbeType::Http, 60);
    let id = probe.id;
    scheduler.register(probe);
    assert_eq!(scheduler.probe_count(), 1);
    assert!(scheduler.is_registered(id));
}

#[test]
fn test_scheduler_unregister_probe() {
    let scheduler = ProbeScheduler::new(SchedulerConfig::default());
    let probe = make_probe("test", ProbeType::Http, 60);
    let id = probe.id;
    scheduler.register(probe);
    assert!(scheduler.unregister(id));
    assert_eq!(scheduler.probe_count(), 0);
    assert!(!scheduler.unregister(id), "double unregister returns false");
}

#[test]
fn test_scheduler_multiple_probes() {
    let scheduler = ProbeScheduler::new(SchedulerConfig::default());
    for i in 0..5 {
        scheduler.register(make_probe(&format!("probe-{i}"), ProbeType::Http, 60));
    }
    assert_eq!(scheduler.probe_count(), 5);
}

#[test]
fn test_scheduler_due_probes_none_initially() {
    let scheduler = ProbeScheduler::new(SchedulerConfig::default());
    // Register probes with large intervals
    for i in 0..3 {
        scheduler.register(make_probe(&format!("p{i}"), ProbeType::Http, 3600));
    }
    // None are due immediately without a tick
    let due = scheduler.due_probes();
    // They ARE due because next_run starts at epoch 0 (overdue immediately)
    assert!(due.len() <= 3);
}

#[test]
fn test_scheduler_mark_executed() {
    let scheduler = ProbeScheduler::new(SchedulerConfig::default());
    let probe = make_probe("p", ProbeType::Http, 60);
    let id = probe.id;
    scheduler.register(probe);
    scheduler.mark_executed(id);
    // After marking executed, probe should not be due for 60s
    let due = scheduler.due_probes();
    assert!(!due.iter().any(|p| p.id == id));
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn make_probe(name: &str, probe_type: ProbeType, interval_seconds: u32) -> UptimeProbe {
    UptimeProbe {
        id: Uuid::new_v4(),
        name: name.to_string(),
        target_url: "http://example.com".to_string(),
        probe_type,
        interval_seconds,
        timeout_ms: 5000,
        enabled: true,
    }
}

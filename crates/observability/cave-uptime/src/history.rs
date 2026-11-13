// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Heartbeat history store and uptime window statistics.
//!
//! Maps to Uptime Kuma's heartbeat storage and uptime calculation logic.

use crate::models::ProbeResult;
use chrono::Utc;
use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;
use uuid::Uuid;

// ─── Window enum ─────────────────────────────────────────────────────────────

/// Time windows for uptime statistics, matching Uptime Kuma's defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UptimeWindow {
    Hours24,
    Days7,
    Days30,
}

impl UptimeWindow {
    /// Human-readable label (e.g. "24h").
    pub fn label(&self) -> &'static str {
        match self {
            UptimeWindow::Hours24 => "24h",
            UptimeWindow::Days7 => "7d",
            UptimeWindow::Days30 => "30d",
        }
    }

    /// Window size in seconds.
    pub fn seconds(&self) -> i64 {
        match self {
            UptimeWindow::Hours24 => 24 * 3600,
            UptimeWindow::Days7 => 7 * 24 * 3600,
            UptimeWindow::Days30 => 30 * 24 * 3600,
        }
    }
}

// ─── Window stats ─────────────────────────────────────────────────────────────

/// Aggregated statistics for a single uptime window.
#[derive(Debug, Clone)]
pub struct WindowStats {
    pub window: UptimeWindow,
    pub total_checks: u64,
    pub successful_checks: u64,
    pub failed_checks: u64,
    pub uptime_pct: f64,
    pub avg_latency_ms: f64,
    pub min_latency_ms: u64,
    pub max_latency_ms: u64,
}

/// Compute `WindowStats` from a slice of `ProbeResult`s.
///
/// Only results within the window boundary (relative to `Utc::now()`) are
/// included. Results outside the window are silently skipped.
pub fn compute_window_stats(results: &[ProbeResult], window: UptimeWindow) -> WindowStats {
    let cutoff = Utc::now().timestamp() - window.seconds();
    let in_window: Vec<&ProbeResult> = results
        .iter()
        .filter(|r| r.checked_at.timestamp() >= cutoff)
        .collect();

    let total = in_window.len() as u64;
    if total == 0 {
        return WindowStats {
            window,
            total_checks: 0,
            successful_checks: 0,
            failed_checks: 0,
            uptime_pct: 100.0,
            avg_latency_ms: 0.0,
            min_latency_ms: 0,
            max_latency_ms: 0,
        };
    }

    let successful = in_window.iter().filter(|r| r.success).count() as u64;
    let failed = total - successful;

    let latencies: Vec<u64> = in_window
        .iter()
        .filter(|r| r.success)
        .map(|r| r.latency_ms)
        .collect();

    let avg_latency = if latencies.is_empty() {
        0.0
    } else {
        latencies.iter().sum::<u64>() as f64 / latencies.len() as f64
    };
    let min_latency = latencies.iter().copied().min().unwrap_or(0);
    let max_latency = latencies.iter().copied().max().unwrap_or(0);

    WindowStats {
        window,
        total_checks: total,
        successful_checks: successful,
        failed_checks: failed,
        uptime_pct: successful as f64 / total as f64 * 100.0,
        avg_latency_ms: avg_latency,
        min_latency_ms: min_latency,
        max_latency_ms: max_latency,
    }
}

// ─── Probe history (per-probe record) ────────────────────────────────────────

/// Per-probe heartbeat history used by the status page display.
#[derive(Debug, Clone)]
pub struct ProbeHistory {
    pub probe_id: Uuid,
    pub results: Vec<ProbeResult>,
}

// ─── HeartbeatStore ───────────────────────────────────────────────────────────

/// Thread-safe in-memory heartbeat store.
///
/// Maintains a bounded ring buffer of `ProbeResult`s per probe, matching
/// Uptime Kuma's heartbeat storage strategy.
pub struct HeartbeatStore {
    capacity: usize,
    inner: RwLock<HashMap<Uuid, VecDeque<ProbeResult>>>,
}

impl HeartbeatStore {
    /// Create a new store with `capacity` entries per probe.
    pub fn new(capacity: usize) -> Self {
        HeartbeatStore {
            capacity,
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Record a new probe result.
    pub fn record(&self, result: ProbeResult) {
        let mut guard = self.inner.write().unwrap();
        let queue = guard.entry(result.probe_id).or_insert_with(VecDeque::new);
        if queue.len() >= self.capacity {
            queue.pop_front(); // evict oldest
        }
        queue.push_back(result);
    }

    /// Retrieve the last `limit` results for a probe (most recent last).
    pub fn get_history(&self, probe_id: Uuid, limit: usize) -> Vec<ProbeResult> {
        let guard = self.inner.read().unwrap();
        match guard.get(&probe_id) {
            None => Vec::new(),
            Some(q) => {
                let skip = q.len().saturating_sub(limit);
                q.iter().skip(skip).cloned().collect()
            }
        }
    }

    /// Compute window statistics for a probe from stored heartbeats.
    pub fn window_stats(&self, probe_id: Uuid, window: UptimeWindow) -> WindowStats {
        let history = self.get_history(probe_id, self.capacity);
        compute_window_stats(&history, window)
    }

    /// All windows stats (24h/7d/30d) for a probe.
    pub fn all_window_stats(&self, probe_id: Uuid) -> [WindowStats; 3] {
        [
            self.window_stats(probe_id, UptimeWindow::Hours24),
            self.window_stats(probe_id, UptimeWindow::Days7),
            self.window_stats(probe_id, UptimeWindow::Days30),
        ]
    }

    /// Number of results stored for a probe.
    pub fn count(&self, probe_id: Uuid) -> usize {
        self.inner
            .read()
            .unwrap()
            .get(&probe_id)
            .map(|q| q.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    fn result(probe_id: Uuid, success: bool, latency_ms: u64, secs_ago: i64) -> ProbeResult {
        ProbeResult {
            probe_id,
            success,
            latency_ms,
            status_code: if success { Some(200) } else { None },
            error: None,
            checked_at: Utc::now() - ChronoDuration::seconds(secs_ago),
        }
    }

    #[test]
    fn window_seconds() {
        assert_eq!(UptimeWindow::Hours24.seconds(), 86400);
        assert_eq!(UptimeWindow::Days7.seconds(), 604800);
        assert_eq!(UptimeWindow::Days30.seconds(), 2592000);
    }

    #[test]
    fn compute_stats_all_success() {
        let id = Uuid::new_v4();
        let results: Vec<_> = (0..10).map(|i| result(id, true, 100, i * 60)).collect();
        let s = compute_window_stats(&results, UptimeWindow::Hours24);
        assert_eq!(s.total_checks, 10);
        assert_eq!(s.failed_checks, 0);
        assert!((s.uptime_pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn store_capacity_enforced() {
        let id = Uuid::new_v4();
        let store = HeartbeatStore::new(3);
        for i in 0..10 {
            store.record(result(id, true, 10, i));
        }
        assert_eq!(store.count(id), 3);
    }

    #[test]
    fn store_history_limit() {
        let id = Uuid::new_v4();
        let store = HeartbeatStore::new(100);
        for i in 0..20 {
            store.record(result(id, true, 10, i));
        }
        let h = store.get_history(id, 5);
        assert_eq!(h.len(), 5);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus metrics for cave-local-llm.
//!
//! Phase 1 metrics:
//!   cave_local_llm_drafts_generated_total   — counter
//!   cave_local_llm_drafts_failed_total      — counter
//!   cave_local_llm_draft_duration_seconds   — histogram (0.1 s … 120 s)
//!
//! Phase 3 daemon metrics:
//!   cave_local_llm_daemon_ticks_total              — counter
//!   cave_local_llm_tier1_commits_total{crate}      — counter
//!   cave_local_llm_tier2_escalations_total{crate,error_kind} — counter
//!   cave_local_llm_queue_items_by_status           — gauge set (pending/in_progress/done/stuck)
//!   cave_local_llm_daemon_sleep_duration_seconds   — histogram

use std::collections::HashMap;
use std::sync::Mutex;

use prometheus_client::{
    metrics::{counter::Counter, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};

// ── Phase 1: DraftMetrics ─────────────────────────────────────────────────────

pub struct DraftMetrics {
    /// Total drafts successfully written to disk.
    pub drafts_generated_total: Counter,
    /// Total draft generation attempts that ended in error.
    pub drafts_failed_total: Counter,
    /// End-to-end draft generation time in seconds (manifest read → file write).
    pub draft_duration_seconds: Histogram,
}

impl DraftMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let drafts_generated_total = Counter::default();
        let drafts_failed_total = Counter::default();
        let draft_duration_seconds = Histogram::new(
            [0.1_f64, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0].into_iter(),
        );

        registry.register(
            "cave_local_llm_drafts_generated",
            "Total drafts successfully generated and written to docs/drafts/",
            drafts_generated_total.clone(),
        );
        registry.register(
            "cave_local_llm_drafts_failed",
            "Total draft generation attempts that ended in error",
            drafts_failed_total.clone(),
        );
        registry.register(
            "cave_local_llm_draft_duration_seconds",
            "End-to-end draft generation latency in seconds",
            draft_duration_seconds.clone(),
        );

        Self { drafts_generated_total, drafts_failed_total, draft_duration_seconds }
    }
}

// ── Phase 3: label-valued counters (manual map) ───────────────────────────────

/// Simple counter map keyed by a single label value, backed by a mutex-protected
/// `HashMap<String, Counter>`.  Registers lazily on first increment.
pub struct LabelCounter {
    pub inner: Mutex<HashMap<String, Counter>>,
}

impl LabelCounter {
    pub fn new(_metric_name: &'static str, _help: &'static str) -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    pub fn inc_crate(&self, crate_name: &str) {
        let mut map = self.inner.lock().unwrap();
        map.entry(crate_name.to_string()).or_default().inc();
    }
}

/// Counter keyed by (crate, error_kind).
pub struct Label2Counter {
    inner: Mutex<HashMap<(String, String), Counter>>,
}

impl Label2Counter {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    pub fn inc_crate_kind(&self, crate_name: &str, error_kind: &str) {
        let mut map = self.inner.lock().unwrap();
        map.entry((crate_name.to_string(), error_kind.to_string()))
            .or_default()
            .inc();
    }

    pub fn get(&self, crate_name: &str, error_kind: &str) -> u64 {
        let map = self.inner.lock().unwrap();
        map.get(&(crate_name.to_string(), error_kind.to_string()))
            .map(|c| c.get())
            .unwrap_or(0)
    }
}

impl Default for Label2Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// Gauge set for queue item counts by status.
pub struct QueueStatusGauge {
    pub pending: Gauge,
    pub in_progress: Gauge,
    pub done: Gauge,
    pub stuck: Gauge,
}

impl QueueStatusGauge {
    pub fn new(registry: &mut Registry) -> Self {
        let pending: Gauge = Gauge::default();
        let in_progress: Gauge = Gauge::default();
        let done: Gauge = Gauge::default();
        let stuck: Gauge = Gauge::default();

        registry.register(
            "cave_local_llm_queue_pending",
            "Number of queue items with status=pending",
            pending.clone(),
        );
        registry.register(
            "cave_local_llm_queue_in_progress",
            "Number of queue items with status=in_progress",
            in_progress.clone(),
        );
        registry.register(
            "cave_local_llm_queue_done",
            "Number of queue items with status=done",
            done.clone(),
        );
        registry.register(
            "cave_local_llm_queue_stuck",
            "Number of queue items with status=stuck",
            stuck.clone(),
        );

        Self { pending, in_progress, done, stuck }
    }

    pub fn set_pending(&self, v: i64) {
        self.pending.set(v);
    }

    pub fn set_in_progress(&self, v: i64) {
        self.in_progress.set(v);
    }

    pub fn set_done(&self, v: i64) {
        self.done.set(v);
    }

    pub fn set_stuck(&self, v: i64) {
        self.stuck.set(v);
    }
}

// ── Phase 3: DaemonMetrics ────────────────────────────────────────────────────

pub struct DaemonMetrics {
    /// Total daemon ticks processed.
    pub daemon_ticks_total: Counter,
    /// Successful tier-1 commits keyed by crate name.
    pub tier1_commits_total: LabelCounter,
    /// Tier-2 escalations keyed by (crate, error_kind).
    pub tier2_escalations_total: Label2Counter,
    /// Queue item counts by status.
    pub queue_items_by_status: QueueStatusGauge,
    /// Histogram of per-tick processing time (seconds).
    pub daemon_sleep_duration_seconds: Histogram,
}

impl DaemonMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let daemon_ticks_total = Counter::default();
        let daemon_sleep_duration_seconds = Histogram::new(
            [0.5_f64, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0].into_iter(),
        );

        registry.register(
            "cave_local_llm_daemon_ticks",
            "Total daemon ticks executed",
            daemon_ticks_total.clone(),
        );
        registry.register(
            "cave_local_llm_daemon_sleep_duration_seconds",
            "Per-tick processing time in seconds",
            daemon_sleep_duration_seconds.clone(),
        );

        let queue_items_by_status = QueueStatusGauge::new(registry);

        Self {
            daemon_ticks_total,
            tier1_commits_total: LabelCounter::new(
                "cave_local_llm_tier1_commits",
                "Successful tier-1 commits by crate",
            ),
            tier2_escalations_total: Label2Counter::new(),
            queue_items_by_status,
            daemon_sleep_duration_seconds,
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::registry::Registry;

    // ── DraftMetrics ──────────────────────────────────────────────────────────

    #[test]
    fn test_counters_increment() {
        let mut reg = Registry::default();
        let m = DraftMetrics::new(&mut reg);

        m.drafts_generated_total.inc();
        m.drafts_generated_total.inc();
        m.drafts_failed_total.inc();

        assert_eq!(m.drafts_generated_total.get(), 2);
        assert_eq!(m.drafts_failed_total.get(), 1);
    }

    #[test]
    fn test_histogram_observe_does_not_panic() {
        let mut reg = Registry::default();
        let m = DraftMetrics::new(&mut reg);
        m.draft_duration_seconds.observe(1.5);
        m.draft_duration_seconds.observe(45.0);
        m.draft_duration_seconds.observe(0.05);
    }

    #[test]
    fn test_registry_encodes_draft_metrics() {
        let mut reg = Registry::default();
        let m = DraftMetrics::new(&mut reg);
        m.drafts_generated_total.inc();
        m.draft_duration_seconds.observe(3.0);

        let mut output = String::new();
        prometheus_client::encoding::text::encode(&mut output, &reg).unwrap();
        assert!(output.contains("cave_local_llm_drafts_generated_total"));
        assert!(output.contains("cave_local_llm_drafts_failed_total"));
        assert!(output.contains("cave_local_llm_draft_duration_seconds"));
    }

    // ── DaemonMetrics ─────────────────────────────────────────────────────────

    #[test]
    fn test_daemon_ticks_counter_increments() {
        let mut reg = Registry::default();
        let m = DaemonMetrics::new(&mut reg);
        m.daemon_ticks_total.inc();
        m.daemon_ticks_total.inc();
        assert_eq!(m.daemon_ticks_total.get(), 2);
    }

    #[test]
    fn test_tier1_commits_per_crate() {
        let mut reg = Registry::default();
        let m = DaemonMetrics::new(&mut reg);
        m.tier1_commits_total.inc_crate("cave-secrets");
        m.tier1_commits_total.inc_crate("cave-secrets");
        m.tier1_commits_total.inc_crate("cave-auth");
        let map = m.tier1_commits_total.inner.lock().unwrap();
        assert_eq!(map["cave-secrets"].get(), 2);
        assert_eq!(map["cave-auth"].get(), 1);
    }

    #[test]
    fn test_tier2_escalations_per_crate_and_kind() {
        let mut reg = Registry::default();
        let m = DaemonMetrics::new(&mut reg);
        m.tier2_escalations_total.inc_crate_kind("cave-events", "test_fail");
        m.tier2_escalations_total.inc_crate_kind("cave-events", "test_fail");
        m.tier2_escalations_total.inc_crate_kind("cave-events", "timeout");
        assert_eq!(m.tier2_escalations_total.get("cave-events", "test_fail"), 2);
        assert_eq!(m.tier2_escalations_total.get("cave-events", "timeout"), 1);
        assert_eq!(m.tier2_escalations_total.get("cave-events", "compile_fail"), 0);
    }

    #[test]
    fn test_queue_status_gauge_set_and_read() {
        let mut reg = Registry::default();
        let m = DaemonMetrics::new(&mut reg);
        m.queue_items_by_status.set_pending(5);
        m.queue_items_by_status.set_in_progress(2);
        m.queue_items_by_status.set_done(10);
        m.queue_items_by_status.set_stuck(1);
        assert_eq!(m.queue_items_by_status.pending.get(), 5);
        assert_eq!(m.queue_items_by_status.in_progress.get(), 2);
        assert_eq!(m.queue_items_by_status.done.get(), 10);
        assert_eq!(m.queue_items_by_status.stuck.get(), 1);
    }

    #[test]
    fn test_daemon_histogram_observe() {
        let mut reg = Registry::default();
        let m = DaemonMetrics::new(&mut reg);
        m.daemon_sleep_duration_seconds.observe(0.8);
        m.daemon_sleep_duration_seconds.observe(120.0);
    }

    #[test]
    fn test_registry_encodes_daemon_metrics() {
        let mut reg = Registry::default();
        let m = DaemonMetrics::new(&mut reg);
        m.daemon_ticks_total.inc();
        m.queue_items_by_status.set_pending(3);

        let mut output = String::new();
        prometheus_client::encoding::text::encode(&mut output, &reg).unwrap();
        assert!(output.contains("cave_local_llm_daemon_ticks_total"));
        assert!(output.contains("cave_local_llm_queue_pending"));
        assert!(output.contains("cave_local_llm_daemon_sleep_duration_seconds"));
    }
}

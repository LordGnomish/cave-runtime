// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Probe scheduler — interval-based dispatch engine.
//!
//! Maps to Uptime Kuma's internal heartbeat scheduler that fires each
//! monitor at its configured interval.

use crate::models::UptimeProbe;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

// ─── SchedulerConfig ─────────────────────────────────────────────────────────

/// Configuration for the probe scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum number of probes executing concurrently.
    pub max_concurrent_probes: usize,
    /// Random jitter added to each interval (ms) to prevent thundering herd.
    pub jitter_ms: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        SchedulerConfig {
            max_concurrent_probes: 100,
            jitter_ms: 1000,
        }
    }
}

// ─── ProbeEntry ───────────────────────────────────────────────────────────────

/// Internal entry tracking a probe and its schedule.
#[derive(Debug, Clone)]
struct ProbeEntry {
    probe: UptimeProbe,
    /// Unix timestamp (seconds) when the probe should next run.
    next_run_at: i64,
}

// ─── ProbeScheduler ───────────────────────────────────────────────────────────

/// Interval-based probe scheduler.
///
/// Maintains a registry of probes and tracks when each is next due to run.
/// The actual async dispatch is done externally via a tokio task that calls
/// `due_probes()` then `mark_executed()` after each run.
pub struct ProbeScheduler {
    config: SchedulerConfig,
    entries: RwLock<HashMap<Uuid, ProbeEntry>>,
}

impl ProbeScheduler {
    /// Create a new scheduler with the given configuration.
    pub fn new(config: SchedulerConfig) -> Self {
        ProbeScheduler {
            config,
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Register a probe. Starts as immediately due (next_run_at = 0).
    pub fn register(&self, probe: UptimeProbe) {
        let entry = ProbeEntry {
            probe: probe.clone(),
            next_run_at: 0, // immediately due on first registration
        };
        self.entries.write().unwrap().insert(probe.id, entry);
    }

    /// Unregister a probe. Returns `true` if it existed.
    pub fn unregister(&self, id: Uuid) -> bool {
        self.entries.write().unwrap().remove(&id).is_some()
    }

    /// Update a probe's configuration in the scheduler.
    pub fn update(&self, probe: UptimeProbe) -> bool {
        let mut guard = self.entries.write().unwrap();
        if let Some(entry) = guard.get_mut(&probe.id) {
            let interval = probe.interval_seconds as i64;
            entry.probe = probe;
            // Preserve existing next_run_at but cap it to now + interval
            let now = chrono::Utc::now().timestamp();
            if entry.next_run_at > now + interval {
                entry.next_run_at = now + interval;
            }
            true
        } else {
            false
        }
    }

    /// Returns all probes that are currently due to run.
    pub fn due_probes(&self) -> Vec<UptimeProbe> {
        let now = chrono::Utc::now().timestamp();
        self.entries
            .read()
            .unwrap()
            .values()
            .filter(|e| e.probe.enabled && e.next_run_at <= now)
            .map(|e| e.probe.clone())
            .collect()
    }

    /// Mark a probe as having just executed. Schedules the next run at
    /// `now + interval_seconds`.
    pub fn mark_executed(&self, id: Uuid) {
        let now = chrono::Utc::now().timestamp();
        if let Some(entry) = self.entries.write().unwrap().get_mut(&id) {
            entry.next_run_at = now + entry.probe.interval_seconds as i64;
        }
    }

    /// Number of registered probes.
    pub fn probe_count(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Returns `true` if a probe with the given ID is registered.
    pub fn is_registered(&self, id: Uuid) -> bool {
        self.entries.read().unwrap().contains_key(&id)
    }

    /// Returns the next scheduled run time (unix timestamp) for a probe.
    pub fn next_run_at(&self, id: Uuid) -> Option<i64> {
        self.entries
            .read()
            .unwrap()
            .get(&id)
            .map(|e| e.next_run_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProbeType, UptimeProbe};

    fn probe(interval_seconds: u32) -> UptimeProbe {
        UptimeProbe {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            target_url: "http://example.com".to_string(),
            probe_type: ProbeType::Http,
            interval_seconds,
            timeout_ms: 5000,
            enabled: true,
        }
    }

    #[test]
    fn register_and_unregister() {
        let s = ProbeScheduler::new(SchedulerConfig::default());
        let p = probe(60);
        let id = p.id;
        s.register(p);
        assert_eq!(s.probe_count(), 1);
        assert!(s.unregister(id));
        assert_eq!(s.probe_count(), 0);
    }

    #[test]
    fn due_immediately_after_register() {
        let s = ProbeScheduler::new(SchedulerConfig::default());
        let p = probe(60);
        let id = p.id;
        s.register(p);
        let due = s.due_probes();
        assert!(due.iter().any(|d| d.id == id));
    }

    #[test]
    fn not_due_after_mark_executed() {
        let s = ProbeScheduler::new(SchedulerConfig::default());
        let p = probe(3600); // 1 hour interval
        let id = p.id;
        s.register(p);
        s.mark_executed(id);
        let due = s.due_probes();
        assert!(!due.iter().any(|d| d.id == id));
    }
}

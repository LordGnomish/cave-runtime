// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Status page model.
//!
//! Maps to Uptime Kuma's status page builder — a public-facing page that
//! shows the operational state of monitored services.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Monitor status enum ──────────────────────────────────────────────────────

/// Operational status of a single monitored service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonitorStatus {
    /// All checks passing.
    Up,
    /// Latest check failed.
    Down,
    /// No check has run yet.
    Pending,
    /// Maintenance window active.
    Maintenance,
}

impl MonitorStatus {
    /// Lowercase label used in status page HTML / JSON.
    pub fn label(&self) -> &'static str {
        match self {
            MonitorStatus::Up => "up",
            MonitorStatus::Down => "down",
            MonitorStatus::Pending => "pending",
            MonitorStatus::Maintenance => "maintenance",
        }
    }

    /// Returns `true` if the monitor is currently UP.
    pub fn is_up(&self) -> bool {
        matches!(self, MonitorStatus::Up)
    }
}

// ─── ProbeStatusSummary ───────────────────────────────────────────────────────

/// Per-probe status summary shown on the status page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeStatusSummary {
    pub probe_id: Uuid,
    pub name: String,
    pub status: MonitorStatus,
    pub uptime_24h: f64,
    pub avg_latency_ms: f64,
    pub last_check_ms: u64,
}

// ─── StatusPage ───────────────────────────────────────────────────────────────

/// Status page entry (wraps a probe summary with optional description).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPageEntry {
    pub summary: ProbeStatusSummary,
    pub description: Option<String>,
}

/// A rendered status page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPage {
    pub title: String,
    pub description: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<StatusPageEntry>,
}

impl StatusPage {
    /// Returns `true` if every entry is either `Up` or `Maintenance`.
    pub fn all_operational(&self) -> bool {
        self.entries.iter().all(|e| {
            matches!(
                e.summary.status,
                MonitorStatus::Up | MonitorStatus::Maintenance
            )
        })
    }

    /// Count of probes currently `Down`.
    pub fn down_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.summary.status == MonitorStatus::Down)
            .count()
    }

    /// Count of probes currently `Up`.
    pub fn up_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.summary.status == MonitorStatus::Up)
            .count()
    }

    /// Overall uptime percentage across all UP/DOWN probes (24h window average).
    pub fn overall_uptime_24h(&self) -> f64 {
        let relevant: Vec<f64> = self
            .entries
            .iter()
            .filter(|e| !matches!(e.summary.status, MonitorStatus::Pending))
            .map(|e| e.summary.uptime_24h)
            .collect();
        if relevant.is_empty() {
            return 100.0;
        }
        relevant.iter().sum::<f64>() / relevant.len() as f64
    }
}

/// Build a `StatusPage` from a list of probe summaries.
pub fn build_status_page(title: &str, summaries: Vec<ProbeStatusSummary>) -> StatusPage {
    let entries = summaries
        .into_iter()
        .map(|s| StatusPageEntry {
            summary: s,
            description: None,
        })
        .collect();

    StatusPage {
        title: title.to_string(),
        description: None,
        generated_at: Utc::now(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_roundtrip() {
        assert_eq!(MonitorStatus::Up.label(), "up");
        assert_eq!(MonitorStatus::Down.label(), "down");
    }

    #[test]
    fn all_operational_when_all_up() {
        let page = build_status_page(
            "test",
            vec![
                make_summary(MonitorStatus::Up),
                make_summary(MonitorStatus::Up),
            ],
        );
        assert!(page.all_operational());
    }

    #[test]
    fn not_operational_when_one_down() {
        let page = build_status_page(
            "test",
            vec![
                make_summary(MonitorStatus::Up),
                make_summary(MonitorStatus::Down),
            ],
        );
        assert!(!page.all_operational());
        assert_eq!(page.down_count(), 1);
    }

    fn make_summary(status: MonitorStatus) -> ProbeStatusSummary {
        ProbeStatusSummary {
            probe_id: Uuid::new_v4(),
            name: "x".to_string(),
            status,
            uptime_24h: 100.0,
            avg_latency_ms: 50.0,
            last_check_ms: 50,
        }
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Job progress tracker — port of `pkg/sources/job_progress.go` +
//! `job_progress_hook.go`. Used by the engine to surface scan progress to
//! the Portal websocket + Prometheus.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct JobProgress {
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub units_total: u64,
    pub units_completed: u64,
    pub chunks_emitted: u64,
    pub findings_emitted: u64,
    pub findings_verified: u64,
    pub errors: Vec<String>,
}

impl JobProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self) {
        self.started_at = Some(Utc::now());
    }

    pub fn finish(&mut self) {
        self.completed_at = Some(Utc::now());
    }

    pub fn record_chunk(&mut self) {
        self.chunks_emitted += 1;
    }

    pub fn record_finding(&mut self, verified: bool) {
        self.findings_emitted += 1;
        if verified {
            self.findings_verified += 1;
        }
    }

    pub fn record_error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    pub fn percent(&self) -> f64 {
        if self.units_total == 0 {
            return 0.0;
        }
        (self.units_completed as f64 / self.units_total as f64) * 100.0
    }

    pub fn done(&self) -> bool {
        self.units_total > 0 && self.units_completed >= self.units_total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_zero_when_no_total() {
        let j = JobProgress::new();
        assert_eq!(j.percent(), 0.0);
        assert!(!j.done());
    }

    #[test]
    fn percent_half() {
        let j = JobProgress {
            units_total: 10,
            units_completed: 5,
            ..Default::default()
        };
        assert_eq!(j.percent(), 50.0);
    }

    #[test]
    fn done_when_all_units_complete() {
        let j = JobProgress {
            units_total: 3,
            units_completed: 3,
            ..Default::default()
        };
        assert!(j.done());
    }

    #[test]
    fn record_finding_increments_counters() {
        let mut j = JobProgress::new();
        j.record_finding(true);
        j.record_finding(false);
        assert_eq!(j.findings_emitted, 2);
        assert_eq!(j.findings_verified, 1);
    }

    #[test]
    fn start_and_finish_set_timestamps() {
        let mut j = JobProgress::new();
        j.start();
        assert!(j.started_at.is_some());
        j.finish();
        assert!(j.completed_at.is_some());
    }
}

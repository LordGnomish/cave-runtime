// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sift "Kube Crashes" check — crashloop detection.
//!
//! Upstream: grafana/sift — the Kube-Crashes analyzer queries Prometheus
//! for pods whose restart counter is climbing and flags crashlooping
//! workloads. cave-forensics observes the same churn directly: a
//! crashlooping container produces a burst of abnormal [`ProcessExit`]
//! events (non-zero status or a fatal signal) for the same entity.
//!
//! Detection is a sliding window: for each entity key (container id, or
//! the binary path when uncontainerized), find the densest `window_secs`
//! span over its abnormal-exit timestamps. If that span holds at least
//! `min_restarts` exits, emit a finding whose severity scales with the
//! restart count.

use crate::events::KernelEvent;
use crate::models::ForensicSeverity;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A detected crashloop for one entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrashLoopFinding {
    /// Container id, or binary path when the process is not containerized.
    pub key: String,
    /// Number of abnormal exits inside the densest window.
    pub restarts: usize,
    /// The sliding-window width that produced this finding (seconds).
    pub window_secs: i64,
    /// Timestamp of the last abnormal exit in the window.
    pub last_exit: DateTime<Utc>,
    /// Severity, escalating with restart count.
    pub severity: ForensicSeverity,
}

/// True if a `ProcessExit` is "abnormal" — a non-zero status code or a
/// terminating signal (SIGKILL/SIGSEGV/OOM-kill all surface here).
fn is_abnormal_exit(status: i32, signal: Option<i32>) -> bool {
    status != 0 || signal.is_some()
}

/// Map a restart count to a severity band.
fn severity_for(restarts: usize) -> ForensicSeverity {
    match restarts {
        0..=3 => ForensicSeverity::Medium,
        4..=6 => ForensicSeverity::High,
        _ => ForensicSeverity::Critical,
    }
}

/// Detect crashloops across an event stream.
///
/// `window_secs` is the sliding-window width; `min_restarts` is the
/// inclusive threshold of abnormal exits within any one window that
/// constitutes a crashloop. Findings are returned sorted by key for
/// determinism.
pub fn detect_crashloops(
    events: &[KernelEvent],
    window_secs: i64,
    min_restarts: usize,
) -> Vec<CrashLoopFinding> {
    // Bucket abnormal-exit timestamps per entity key.
    let mut buckets: BTreeMap<String, Vec<DateTime<Utc>>> = BTreeMap::new();

    for ev in events {
        let KernelEvent::ProcessExit(exit) = ev else {
            continue;
        };
        if !is_abnormal_exit(exit.status, exit.signal) {
            continue;
        }
        let key = exit
            .process
            .container_id
            .clone()
            .unwrap_or_else(|| exit.process.binary.clone());
        buckets.entry(key).or_default().push(exit.observed_at);
    }

    let mut out = Vec::new();
    for (key, mut times) in buckets {
        times.sort();
        if let Some((restarts, last)) = densest_window(&times, window_secs) {
            if restarts >= min_restarts {
                out.push(CrashLoopFinding {
                    key,
                    restarts,
                    window_secs,
                    last_exit: last,
                    severity: severity_for(restarts),
                });
            }
        }
    }
    out
}

/// Two-pointer sweep over sorted timestamps: the maximum number of points
/// contained in any half-open `window_secs` span, plus the last timestamp
/// of that densest span. Returns `None` for an empty slice.
fn densest_window(
    sorted: &[DateTime<Utc>],
    window_secs: i64,
) -> Option<(usize, DateTime<Utc>)> {
    if sorted.is_empty() {
        return None;
    }
    let mut best = 0usize;
    let mut best_last = sorted[0];
    let mut start = 0usize;
    for end in 0..sorted.len() {
        while (sorted[end] - sorted[start]).num_seconds() > window_secs {
            start += 1;
        }
        let count = end - start + 1;
        if count > best {
            best = count;
            best_last = sorted[end];
        }
    }
    Some((best, best_last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_abnormal_exit_rules() {
        assert!(!is_abnormal_exit(0, None));
        assert!(is_abnormal_exit(1, None));
        assert!(is_abnormal_exit(0, Some(9)));
    }

    #[test]
    fn test_severity_bands() {
        assert_eq!(severity_for(3), ForensicSeverity::Medium);
        assert_eq!(severity_for(5), ForensicSeverity::High);
        assert_eq!(severity_for(9), ForensicSeverity::Critical);
    }

    #[test]
    fn test_densest_window_empty_is_none() {
        assert!(densest_window(&[], 60).is_none());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HPA stabilization window — `pkg/controller/podautoscaler/horizontal.go`.
//!
//! Mirrors `stabilizeRecommendationWithBehaviors`. Each HPA stores recent
//! recommendations; the post-stabilization replica count is:
//!
//! * **scale-up direction**: `min(recommendation, min over recs in scale-up window)`
//!   — dampens rapid scale-up.
//! * **scale-down direction**: `max(recommendation, max over recs in scale-down window)`
//!   — dampens rapid scale-down (the more common operator concern).
//!
//! Default windows: scale-up `0s`, scale-down `300s` (5 minutes).
//! See `--horizontal-pod-autoscaler-downscale-stabilization` flag.

use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_SCALE_UP_WINDOW_SEC: u32 = 0;
pub const DEFAULT_SCALE_DOWN_WINDOW_SEC: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    /// Replica count recommended at `timestamp_sec`.
    pub replicas: u32,
    /// Monotonic seconds since some epoch — only relative differences matter.
    pub timestamp_sec: u64,
}

/// Per-HPA bounded recommendation history.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RecommendationHistory {
    by_key: HashMap<String, Vec<Recommendation>>,
    /// Soft cap per key; matches upstream `maxRecommendations = 60` approx.
    max_entries: usize,
}

impl RecommendationHistory {
    pub fn new() -> Self {
        Self {
            by_key: HashMap::new(),
            max_entries: 60,
        }
    }

    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            by_key: HashMap::new(),
            max_entries,
        }
    }

    pub fn record(&mut self, key: &str, rec: Recommendation) {
        let buf = self.by_key.entry(key.to_string()).or_default();
        buf.push(rec);
        if buf.len() > self.max_entries {
            let drop = buf.len() - self.max_entries;
            buf.drain(0..drop);
        }
    }

    pub fn entries(&self, key: &str) -> &[Recommendation] {
        self.by_key.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Apply the stabilization window. `now_sec` is the current monotonic time;
/// only entries with `timestamp_sec + window_sec >= now_sec` participate.
pub fn stabilize(
    history: &RecommendationHistory,
    key: &str,
    direction: Direction,
    proposed: u32,
    window_sec: u32,
    now_sec: u64,
) -> u32 {
    if window_sec == 0 {
        return proposed;
    }
    let window = window_sec as u64;
    let entries = history.entries(key);
    match direction {
        Direction::Up => {
            // Take the minimum recommendation in the window (most conservative scale-up).
            let mut min_rec = proposed;
            for r in entries {
                if r.timestamp_sec + window >= now_sec && r.replicas < min_rec {
                    min_rec = r.replicas;
                }
            }
            min_rec
        }
        Direction::Down => {
            // Take the maximum recommendation in the window (most conservative scale-down).
            let mut max_rec = proposed;
            for r in entries {
                if r.timestamp_sec + window >= now_sec && r.replicas > max_rec {
                    max_rec = r.replicas;
                }
            }
            max_rec
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podautoscaler/horizontal.go",
    "stabilizeRecommendationWithBehaviors",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn h() -> RecommendationHistory {
        RecommendationHistory::new()
    }

    #[test]
    fn empty_history_returns_proposed_unchanged() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-empty"
        );
        let h = h();
        assert_eq!(stabilize(&h, "web", Direction::Up, 6, 60, 100), 6);
        assert_eq!(stabilize(&h, "web", Direction::Down, 2, 60, 100), 2);
    }

    #[test]
    fn zero_window_disables_stabilization() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-zero-window"
        );
        let mut h = h();
        h.record(
            "web",
            Recommendation {
                replicas: 99,
                timestamp_sec: 100,
            },
        );
        assert_eq!(stabilize(&h, "web", Direction::Up, 4, 0, 100), 4);
    }

    #[test]
    fn scale_up_takes_min_within_window() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-up-min"
        );
        let mut h = h();
        // Three recent recs: 5, 4, 6 — min is 4 — proposed 8 is dampened.
        h.record(
            "web",
            Recommendation {
                replicas: 5,
                timestamp_sec: 90,
            },
        );
        h.record(
            "web",
            Recommendation {
                replicas: 4,
                timestamp_sec: 95,
            },
        );
        h.record(
            "web",
            Recommendation {
                replicas: 6,
                timestamp_sec: 99,
            },
        );
        assert_eq!(stabilize(&h, "web", Direction::Up, 8, 30, 100), 4);
    }

    #[test]
    fn scale_down_takes_max_within_window() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-down-max"
        );
        let mut h = h();
        h.record(
            "api",
            Recommendation {
                replicas: 5,
                timestamp_sec: 95,
            },
        );
        h.record(
            "api",
            Recommendation {
                replicas: 9,
                timestamp_sec: 96,
            },
        );
        h.record(
            "api",
            Recommendation {
                replicas: 6,
                timestamp_sec: 99,
            },
        );
        // Proposed scale-down to 2; max in window is 9 → keep 9.
        assert_eq!(stabilize(&h, "api", Direction::Down, 2, 60, 100), 9);
    }

    #[test]
    fn entries_outside_window_are_ignored() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-outside"
        );
        let mut h = h();
        // Aged out: ts=10, window=20, now=100 → 10+20=30 < 100 → ignored.
        h.record(
            "api",
            Recommendation {
                replicas: 99,
                timestamp_sec: 10,
            },
        );
        h.record(
            "api",
            Recommendation {
                replicas: 7,
                timestamp_sec: 95,
            },
        );
        assert_eq!(stabilize(&h, "api", Direction::Down, 3, 20, 100), 7);
    }

    #[test]
    fn entry_at_window_edge_is_included() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-edge"
        );
        let mut h = h();
        // ts=80, window=20, now=100 → 80+20=100 >= 100 → included.
        h.record(
            "api",
            Recommendation {
                replicas: 12,
                timestamp_sec: 80,
            },
        );
        assert_eq!(stabilize(&h, "api", Direction::Down, 3, 20, 100), 12);
    }

    #[test]
    fn separate_keys_have_independent_histories() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-separate-keys"
        );
        let mut h = h();
        h.record(
            "web",
            Recommendation {
                replicas: 20,
                timestamp_sec: 99,
            },
        );
        h.record(
            "api",
            Recommendation {
                replicas: 4,
                timestamp_sec: 99,
            },
        );
        // "web" history doesn't influence "api" stabilization.
        assert_eq!(stabilize(&h, "api", Direction::Down, 2, 60, 100), 4);
    }

    #[test]
    fn ring_buffer_evicts_oldest_when_capacity_exceeded() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-ring-evict"
        );
        let mut h = RecommendationHistory::with_capacity(3);
        for i in 1..=5u32 {
            h.record(
                "web",
                Recommendation {
                    replicas: i,
                    timestamp_sec: i as u64,
                },
            );
        }
        let entries = h.entries("web");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].replicas, 3);
        assert_eq!(entries[2].replicas, 5);
    }

    #[test]
    fn proposed_wins_when_no_in_window_recs_more_extreme() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "stabilizeRecommendationWithBehaviors",
            "tenant-hpa-stab-proposed-extreme"
        );
        let mut h = h();
        h.record(
            "web",
            Recommendation {
                replicas: 5,
                timestamp_sec: 99,
            },
        );
        // Scale-down direction: max(2, 5) = 5; 2 doesn't win because 5 is larger.
        // But if proposed=10 (scale-up direction): we use Up branch → min(10, 5) = 5.
        assert_eq!(stabilize(&h, "web", Direction::Up, 10, 30, 100), 5);
        // Scale-down with proposed=8 vs entry=5 → max=8 → proposed wins.
        assert_eq!(stabilize(&h, "web", Direction::Down, 8, 30, 100), 8);
    }

    #[test]
    fn default_windows_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "defaultDownscaleStabilizationWindow",
            "tenant-hpa-stab-defaults"
        );
        // Upstream defaults: scale-up 0, scale-down 300s.
        assert_eq!(DEFAULT_SCALE_UP_WINDOW_SEC, 0);
        assert_eq!(DEFAULT_SCALE_DOWN_WINDOW_SEC, 300);
    }
}

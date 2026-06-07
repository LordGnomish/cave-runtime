// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tracker-state reader.
//!
//! The autopilot does not invent its own notion of "what's done" — it consumes
//! the same `docs/parity/parity-index.json` that `cave-runtime-tracker` and
//! `cave-home-tracker` emit. That file is a map of crate-name → metrics; the
//! honest completion signal we rank on is `honest_ratio` (the
//! manifest-authored, paperwork-free measure), falling back to `parity_ratio`
//! then `fill_ratio` when a crate predates the honest metric.

use crate::error::{AutopilotError, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// One crate's entry as read from `parity-index.json`. Only the fields the
/// ranker needs are typed; the rest are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct CrateEntry {
    #[serde(default)]
    pub crate_dir: Option<String>,
    #[serde(default)]
    pub honest_ratio: Option<f64>,
    #[serde(default)]
    pub parity_ratio: Option<f64>,
    #[serde(default)]
    pub fill_ratio: Option<f64>,
    #[serde(default)]
    pub cave_src_loc: Option<u64>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub infra_only: Option<bool>,
}

impl CrateEntry {
    /// The completion signal we rank on, in priority order: honest → parity →
    /// fill → 0.0. Clamped to `[0,1]`.
    pub fn effective_completion(&self) -> f64 {
        let v = self
            .honest_ratio
            .or(self.parity_ratio)
            .or(self.fill_ratio)
            .unwrap_or(0.0);
        v.clamp(0.0, 1.0)
    }
}

/// A subsystem promoted out of the raw index for queueing. Carries the name
/// plus everything a dispatched task needs to locate and size the work.
#[derive(Debug, Clone, PartialEq)]
pub struct Subsystem {
    pub name: String,
    pub crate_dir: String,
    pub completion: f64,
    pub src_loc: u64,
    pub tier: String,
    pub upstream: Option<String>,
}

/// Whole tracker state. `BTreeMap` keeps iteration deterministic, which makes
/// ranking stable across runs.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrackerState {
    #[serde(default)]
    pub crates: BTreeMap<String, CrateEntry>,
}

impl TrackerState {
    /// Load + parse a tracker index file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            AutopilotError::Tracker(format!("cannot read {}: {e}", path.display()))
        })?;
        let state: TrackerState = serde_json::from_str(&raw)?;
        Ok(state)
    }

    /// Parse from an in-memory string (used by tests and pipes).
    pub fn from_str(raw: &str) -> Result<Self> {
        Ok(serde_json::from_str(raw)?)
    }

    /// Number of crates tracked.
    pub fn len(&self) -> usize {
        self.crates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.crates.is_empty()
    }

    /// Every crate as a [`Subsystem`], in name order.
    pub fn subsystems(&self) -> Vec<Subsystem> {
        self.crates
            .iter()
            .map(|(name, e)| Subsystem {
                name: name.clone(),
                crate_dir: e
                    .crate_dir
                    .clone()
                    .unwrap_or_else(|| format!("crates/{name}")),
                completion: e.effective_completion(),
                src_loc: e.cave_src_loc.unwrap_or(0),
                tier: e.tier.clone().unwrap_or_else(|| "?".to_string()),
                upstream: e.upstream.clone(),
            })
            .collect()
    }

    /// Subsystems below `threshold`, ranked **most incomplete first**. Ties
    /// broken by larger existing src_loc (more momentum → finish it), then by
    /// name for determinism. `infra_only` crates are excluded — they have no
    /// upstream surface to port.
    pub fn ranked_incomplete(&self, threshold: f64) -> Vec<Subsystem> {
        let mut out: Vec<Subsystem> = self
            .crates
            .iter()
            .filter(|(_, e)| !e.infra_only.unwrap_or(false))
            .filter(|(_, e)| e.effective_completion() < threshold)
            .map(|(name, e)| Subsystem {
                name: name.clone(),
                crate_dir: e
                    .crate_dir
                    .clone()
                    .unwrap_or_else(|| format!("crates/{name}")),
                completion: e.effective_completion(),
                src_loc: e.cave_src_loc.unwrap_or(0),
                tier: e.tier.clone().unwrap_or_else(|| "?".to_string()),
                upstream: e.upstream.clone(),
            })
            .collect();
        out.sort_by(|a, b| {
            a.completion
                .partial_cmp(&b.completion)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.src_loc.cmp(&a.src_loc))
                .then(a.name.cmp(&b.name))
        });
        out
    }

    /// True when *every* (non-infra) tracked crate is at or above `idle`.
    /// Drives the daemon's idle (monitor-only) mode.
    pub fn all_at_or_above(&self, idle: f64) -> bool {
        self.crates
            .iter()
            .filter(|(_, e)| !e.infra_only.unwrap_or(false))
            .all(|(_, e)| e.effective_completion() >= idle)
    }

    /// Mean completion across non-infra crates — surfaced as a gauge.
    pub fn mean_completion(&self) -> f64 {
        let vals: Vec<f64> = self
            .crates
            .values()
            .filter(|e| !e.infra_only.unwrap_or(false))
            .map(|e| e.effective_completion())
            .collect();
        if vals.is_empty() {
            return 1.0;
        }
        vals.iter().sum::<f64>() / vals.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "crates": {
        "cave-acme":   { "honest_ratio": 0.0, "fill_ratio": 0.0, "cave_src_loc": 662, "tier": "D2", "crate_dir": "crates/security/cave-acme", "infra_only": true },
        "cave-etcd":   { "honest_ratio": 0.52, "parity_ratio": 0.52, "cave_src_loc": 970, "tier": "B1", "crate_dir": "crates/compute/cave-etcd", "upstream": "etcd-io/etcd" },
        "cave-net":    { "honest_ratio": 0.98, "cave_src_loc": 5000, "tier": "A1", "crate_dir": "crates/networking/cave-net" },
        "cave-policy": { "honest_ratio": 0.65, "cave_src_loc": 1200, "tier": "C1", "crate_dir": "crates/security/cave-policy" },
        "cave-deploy": { "honest_ratio": 0.65, "cave_src_loc": 1800, "tier": "C1", "crate_dir": "crates/cave-deploy" }
      }
    }"#;

    #[test]
    fn parses_and_counts() {
        let s = TrackerState::from_str(FIXTURE).unwrap();
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn effective_completion_prefers_honest_then_parity_then_fill() {
        let only_fill: CrateEntry =
            serde_json::from_str(r#"{ "fill_ratio": 0.4 }"#).unwrap();
        assert_eq!(only_fill.effective_completion(), 0.4);
        let parity_over_fill: CrateEntry =
            serde_json::from_str(r#"{ "parity_ratio": 0.7, "fill_ratio": 0.4 }"#).unwrap();
        assert_eq!(parity_over_fill.effective_completion(), 0.7);
        let honest_wins: CrateEntry =
            serde_json::from_str(r#"{ "honest_ratio": 0.9, "parity_ratio": 0.7 }"#).unwrap();
        assert_eq!(honest_wins.effective_completion(), 0.9);
    }

    #[test]
    fn ranked_incomplete_excludes_infra_and_complete_orders_worst_first() {
        let s = TrackerState::from_str(FIXTURE).unwrap();
        let ranked = s.ranked_incomplete(0.95);
        // cave-acme is infra_only -> excluded. cave-net 0.98 >= 0.95 -> excluded.
        let names: Vec<&str> = ranked.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-etcd", "cave-deploy", "cave-policy"]);
        // etcd worst (0.52) first; deploy & policy both 0.65 but deploy has more
        // src_loc (1800 > 1200) so it wins the tie.
        assert_eq!(ranked[0].name, "cave-etcd");
        assert_eq!(ranked[1].name, "cave-deploy");
    }

    #[test]
    fn idle_detection() {
        let s = TrackerState::from_str(FIXTURE).unwrap();
        assert!(!s.all_at_or_above(0.95));
        let high = r#"{ "crates": { "a": {"honest_ratio": 0.96}, "b": {"honest_ratio": 1.0} } }"#;
        assert!(TrackerState::from_str(high).unwrap().all_at_or_above(0.95));
    }

    #[test]
    fn mean_excludes_infra() {
        let s = TrackerState::from_str(FIXTURE).unwrap();
        // (0.52 + 0.98 + 0.65 + 0.65) / 4 = 0.70
        assert!((s.mean_completion() - 0.70).abs() < 1e-9);
    }

    #[test]
    fn missing_crate_dir_defaults_to_crates_name() {
        let s = TrackerState::from_str(r#"{ "crates": { "cave-x": {"honest_ratio": 0.1} } }"#).unwrap();
        let r = s.ranked_incomplete(0.95);
        assert_eq!(r[0].crate_dir, "crates/cave-x");
    }
}

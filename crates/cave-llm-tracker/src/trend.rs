// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Benchmark history — reads the sibling `daily-<date>.json` reports a
//! run leaves behind and folds them into a per-model trend series.
//!
//! Each daily report is self-contained (Phase 0 emits independent files);
//! this module is the Phase 1 trender that stitches them together so the
//! portal can draw a quality/throughput line and the opt-in apply path
//! (`apply.rs`) can demand a candidate clear its floor on N *consecutive*
//! days before it is eligible to swap.
//!
//! The date key is the `daily-<date>.json` filename stamp, which is what
//! [`crate::report::DailyReport::write_to_dir`] writes, so a directory of
//! reports sorts chronologically by name.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TrackerResult;
use crate::report::DailyReport;
use crate::selection::SelectionStatus;

/// One model's reading on a single day, distilled from that day's verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendPoint {
    /// `daily-<date>.json` stamp (e.g. `2026-05-21`).
    pub date: String,
    /// `cand.mean_quality - baseline.mean_quality` recorded that day.
    pub quality_delta: f32,
    /// Fractional throughput uplift vs. that day's baseline.
    pub throughput_uplift: f32,
    /// The verdict status the model earned that day.
    pub status: SelectionStatus,
}

/// A single model's chronological trend, ascending by date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTrend {
    pub model_id: String,
    pub points: Vec<TrendPoint>,
}

impl ModelTrend {
    /// Trailing run of `UpgradeCandidate` days counting back from the
    /// most recent point. `0` if the latest day is anything else — this
    /// is the streak the Phase 1 opt-in apply path requires to clear a
    /// floor before a swap becomes eligible.
    pub fn consecutive_upgrade_days(&self) -> u32 {
        let mut streak = 0u32;
        for p in self.points.iter().rev() {
            if p.status == SelectionStatus::UpgradeCandidate {
                streak += 1;
            } else {
                break;
            }
        }
        streak
    }

    /// Most recent point, if any.
    pub fn latest(&self) -> Option<&TrendPoint> {
        self.points.last()
    }
}

/// All models seen across a directory of daily reports.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrendHistory {
    pub models: Vec<ModelTrend>,
}

impl TrendHistory {
    pub fn model(&self, model_id: &str) -> Option<&ModelTrend> {
        self.models.iter().find(|m| m.model_id == model_id)
    }
}

/// Pull the `<date>` out of a `daily-<date>.json` file name.
fn date_from_filename(name: &str) -> Option<String> {
    name.strip_prefix("daily-")
        .and_then(|s| s.strip_suffix(".json"))
        .map(|s| s.to_string())
}

/// Read every `daily-<date>.json` in `dir` and fold its verdicts into a
/// per-model [`TrendHistory`]. Baseline rows are excluded (they are not
/// upgrade candidates). Unreadable / unparseable files are skipped so a
/// single corrupt report never blocks the trender. `latest.json` and any
/// other sibling are ignored — only the dated reports carry a date key.
pub fn load_history(dir: &Path) -> TrackerResult<TrendHistory> {
    // model_id -> (date -> point); BTreeMap keeps both axes sorted.
    let mut by_model: BTreeMap<String, BTreeMap<String, TrendPoint>> = BTreeMap::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // A missing report dir is an empty history, not an error.
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TrendHistory::default());
        }
        Err(e) => return Err(e.into()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(date) = date_from_filename(name) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(report) = serde_json::from_str::<DailyReport>(&text) else {
            continue;
        };
        for v in &report.verdicts {
            if v.status == SelectionStatus::Baseline {
                continue;
            }
            by_model
                .entry(v.model_id.clone())
                .or_default()
                .insert(
                    date.clone(),
                    TrendPoint {
                        date: date.clone(),
                        quality_delta: v.quality_delta,
                        throughput_uplift: v.throughput_uplift,
                        status: v.status,
                    },
                );
        }
    }

    let models = by_model
        .into_iter()
        .map(|(model_id, points)| ModelTrend {
            model_id,
            points: points.into_values().collect(),
        })
        .collect();
    Ok(TrendHistory { models })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{cave_prompts, BenchSnapshot, EvalResult};
    use crate::config::TrackerConfig;
    use crate::poll::PollSummary;
    use crate::report::DailyReport;
    use crate::registry::{Candidate, SourceKind};
    use crate::selection::{baseline_verdict, evaluate, SelectionStatus};

    fn snap(model: &str, quality: f32, bytes: usize, ms: u64) -> BenchSnapshot {
        BenchSnapshot {
            model_id: model.to_string(),
            results: cave_prompts()
                .into_iter()
                .map(|p| EvalResult {
                    prompt_id: p.id.to_string(),
                    category: p.category.to_string(),
                    model_id: model.to_string(),
                    elapsed_ms: ms,
                    response_bytes: bytes,
                    quality,
                    timed_out: false,
                })
                .collect(),
        }
    }

    fn cand(model: &str) -> Candidate {
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: model.to_string(),
            family: model.to_string(),
            license: "Apache-2.0".to_string(),
            vram_gib: 8.0,
            disk_gib: 9.0,
            quant: "Q4_K_M".to_string(),
            upstream_ref: model.to_string(),
            score_hint: None,
        }
    }

    /// Write a daily report to `dir` stamped `date`, with `cand_bench`
    /// bytes/quality so its verdict is deterministic.
    fn write_day(dir: &std::path::Path, date: &str, cand_bytes: usize, cand_quality: f32) {
        let cfg = TrackerConfig::default_config();
        let base = snap(&cfg.baseline.model, 0.5, 400, 1000);
        let c = cand("fast:7b");
        let cb = snap(&c.model_id, cand_quality, cand_bytes, 1000);
        let v = evaluate(&cfg, &c, &cb, &base);
        let report = DailyReport::assemble(
            &cfg,
            PollSummary::from_seed_only(),
            base,
            vec![cb],
            vec![baseline_verdict(&cfg.baseline.model), v],
        );
        report.write_to_dir(dir, date).expect("write day");
    }

    #[test]
    fn empty_dir_yields_empty_history() {
        let dir = tempfile::tempdir().unwrap();
        let h = load_history(dir.path()).expect("load");
        assert!(h.models.is_empty());
    }

    #[test]
    fn two_upgrade_days_give_two_points_and_streak_two() {
        let dir = tempfile::tempdir().unwrap();
        // Both days: candidate ~3x throughput → UpgradeCandidate.
        write_day(dir.path(), "2026-05-20", 1200, 0.5);
        write_day(dir.path(), "2026-05-21", 1200, 0.5);
        let h = load_history(dir.path()).expect("load");
        let m = h.model("fast:7b").expect("fast:7b tracked");
        assert_eq!(m.points.len(), 2);
        assert_eq!(m.consecutive_upgrade_days(), 2);
    }

    #[test]
    fn points_are_sorted_ascending_by_date() {
        let dir = tempfile::tempdir().unwrap();
        write_day(dir.path(), "2026-05-21", 1200, 0.5);
        write_day(dir.path(), "2026-05-19", 1200, 0.5);
        write_day(dir.path(), "2026-05-20", 1200, 0.5);
        let h = load_history(dir.path()).expect("load");
        let m = h.model("fast:7b").unwrap();
        let dates: Vec<&str> = m.points.iter().map(|p| p.date.as_str()).collect();
        assert_eq!(dates, ["2026-05-19", "2026-05-20", "2026-05-21"]);
    }

    #[test]
    fn streak_resets_when_latest_day_is_below() {
        let dir = tempfile::tempdir().unwrap();
        // day1 upgrade (fast), day2 below (barely beats → Below).
        write_day(dir.path(), "2026-05-20", 1200, 0.5);
        write_day(dir.path(), "2026-05-21", 405, 0.50);
        let h = load_history(dir.path()).expect("load");
        let m = h.model("fast:7b").unwrap();
        assert_eq!(m.points.last().unwrap().status, SelectionStatus::Below);
        assert_eq!(m.consecutive_upgrade_days(), 0);
    }

    #[test]
    fn baseline_rows_are_not_tracked_as_models() {
        let dir = tempfile::tempdir().unwrap();
        write_day(dir.path(), "2026-05-21", 1200, 0.5);
        let h = load_history(dir.path()).expect("load");
        assert!(h.model("qwen3.6:35b-a3b-coding-mxfp8").is_none());
    }
}

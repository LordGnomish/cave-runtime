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

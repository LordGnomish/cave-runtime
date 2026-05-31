// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cost × quality matrix — joins each candidate's bench quality and
//! throughput against its VRAM/disk cost and places it in one of four
//! quadrants relative to the current baseline.
//!
//! This is the decision surface the portal renders and a human reads
//! before opting into a Phase 1 swap: a model in the **SweetSpot**
//! quadrant is both cheaper and at-least-as-good as the seat today; a
//! **Laggard** is worse on both axes and should never be promoted.
//!
//! Quality comes from `report.candidate_benches`; cost (VRAM/disk) comes
//! from the matching `report.poll.candidates` row; the baseline anchor
//! comes from `report.baseline_bench` + `cfg.baseline`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{cave_prompts, BenchSnapshot, EvalResult};
    use crate::config::TrackerConfig;
    use crate::poll::PollSummary;
    use crate::report::DailyReport;
    use crate::registry::{Candidate, SourceKind};
    use crate::selection::baseline_verdict;
    use std::collections::HashMap;

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

    fn cand(model: &str, vram: f32, disk: f32) -> Candidate {
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: model.to_string(),
            family: model.to_string(),
            license: "Apache-2.0".to_string(),
            vram_gib: vram,
            disk_gib: disk,
            quant: "Q4_K_M".to_string(),
            upstream_ref: model.to_string(),
            score_hint: None,
        }
    }

    /// Report whose poll carries two candidates with explicit VRAM, and
    /// whose benches set their quality. Baseline (cfg) is 22 GiB VRAM,
    /// baseline quality 0.5.
    fn report() -> (TrackerConfig, DailyReport) {
        let cfg = TrackerConfig::default_config();
        let cheap = cand("cheap:7b", 8.0, 9.0); // cheaper than baseline 22
        let pricey = cand("pricey:70b", 48.0, 50.0); // pricier than baseline
        let poll = PollSummary {
            polled_at_utc: "2026-05-21T00:00:00Z".to_string(),
            candidates: vec![cheap.clone(), pricey.clone()],
            per_source_count: HashMap::new(),
            source_errors: HashMap::new(),
        };
        let base = snap(&cfg.baseline.model, 0.5, 400, 1000);
        let cheap_b = snap("cheap:7b", 0.6, 400, 1000); // better quality, cheaper
        let pricey_b = snap("pricey:70b", 0.9, 400, 1000); // better, but pricey
        let report = DailyReport::assemble(
            &cfg,
            poll,
            base,
            vec![cheap_b, pricey_b],
            vec![baseline_verdict(&cfg.baseline.model)],
        );
        (cfg, report)
    }

    #[test]
    fn matrix_has_one_row_per_candidate_bench() {
        let (cfg, r) = report();
        let m = build_matrix(&r, &cfg);
        assert_eq!(m.rows.len(), 2);
    }

    #[test]
    fn cheaper_and_better_lands_in_sweet_spot() {
        let (cfg, r) = report();
        let m = build_matrix(&r, &cfg);
        let row = m.row("cheap:7b").expect("cheap row");
        assert_eq!(row.quadrant, Quadrant::SweetSpot);
    }

    #[test]
    fn better_but_pricier_lands_in_premium() {
        let (cfg, r) = report();
        let m = build_matrix(&r, &cfg);
        let row = m.row("pricey:70b").expect("pricey row");
        assert_eq!(row.quadrant, Quadrant::Premium);
    }

    #[test]
    fn efficiency_is_quality_per_vram_gib() {
        let (cfg, r) = report();
        let m = build_matrix(&r, &cfg);
        let row = m.row("cheap:7b").unwrap();
        // 0.6 quality / 8 GiB = 0.075
        assert!((row.efficiency - 0.075).abs() < 1e-4, "got {}", row.efficiency);
    }

    #[test]
    fn rows_sorted_by_efficiency_descending() {
        let (cfg, r) = report();
        let m = build_matrix(&r, &cfg);
        // cheap: 0.6/8 = 0.075 ; pricey: 0.9/48 = 0.01875 → cheap first.
        assert_eq!(m.rows[0].model_id, "cheap:7b");
        assert!(m.rows[0].efficiency >= m.rows[1].efficiency);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Phase 1 opt-in swap planner.
//!
//! Phase 0 is report-only and the daily cron never calls this. The
//! *human-triggered* `cavectl llm-tracker apply` path does: it asks
//! [`plan_swap`] whether today's best upgrade candidate has cleared a
//! floor on `required_days` **consecutive** days (per [`crate::trend`]),
//! and only when the operator passes `opt_in = true` does the resulting
//! [`SwapPlan`] come back `eligible`.
//!
//! [`apply_swap`] then materialises a *new* [`TrackerConfig`] with the
//! candidate as baseline. The new config still has `auto_swap = false`
//! (it passes `validate()`), because the swap was an explicit operator
//! decision, not an unattended one — the Phase 0 mandate that the cron
//! never swaps on its own is preserved.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{cave_prompts, BenchSnapshot, EvalResult};
    use crate::config::TrackerConfig;
    use crate::poll::PollSummary;
    use crate::report::DailyReport;
    use crate::registry::{Candidate, SourceKind};
    use crate::selection::{baseline_verdict, evaluate};
    use crate::trend::{ModelTrend, TrendHistory, TrendPoint};
    use crate::selection::SelectionStatus;
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

    /// Report whose single candidate `fast:7b` is an UpgradeCandidate.
    fn upgrade_report(cfg: &TrackerConfig) -> DailyReport {
        let base = snap(&cfg.baseline.model, 0.5, 400, 1000);
        let c = cand("fast:7b", 8.0, 9.0);
        let cb = snap("fast:7b", 0.5, 1200, 1000);
        let v = evaluate(cfg, &c, &cb, &base);
        let poll = PollSummary {
            polled_at_utc: "2026-05-21T00:00:00Z".to_string(),
            candidates: vec![c],
            per_source_count: HashMap::new(),
            source_errors: HashMap::new(),
        };
        DailyReport::assemble(
            cfg,
            poll,
            base,
            vec![cb],
            vec![baseline_verdict(&cfg.baseline.model), v],
        )
    }

    fn quiet_report(cfg: &TrackerConfig) -> DailyReport {
        DailyReport::assemble(
            cfg,
            PollSummary::from_seed_only(),
            crate::bench::synth_snapshot(&cfg.baseline.model),
            vec![],
            vec![baseline_verdict(&cfg.baseline.model)],
        )
    }

    fn streak_history(model: &str, days: u32) -> TrendHistory {
        let points = (0..days)
            .map(|i| TrendPoint {
                date: format!("2026-05-{:02}", 10 + i),
                quality_delta: 0.0,
                throughput_uplift: 2.0,
                status: SelectionStatus::UpgradeCandidate,
            })
            .collect();
        TrendHistory {
            models: vec![ModelTrend {
                model_id: model.to_string(),
                points,
            }],
        }
    }

    #[test]
    fn no_upgrade_candidate_means_no_plan() {
        let cfg = TrackerConfig::default_config();
        let h = TrendHistory::default();
        assert!(plan_swap(&cfg, &quiet_report(&cfg), &h, false, 3).is_none());
    }

    #[test]
    fn without_opt_in_plan_is_not_eligible() {
        let cfg = TrackerConfig::default_config();
        let h = streak_history("fast:7b", 5);
        let plan = plan_swap(&cfg, &upgrade_report(&cfg), &h, false, 3).expect("plan");
        assert!(!plan.eligible);
        assert!(plan.reasons.iter().any(|r| r.to_lowercase().contains("opt-in")));
    }

    #[test]
    fn short_streak_is_not_eligible_even_with_opt_in() {
        let cfg = TrackerConfig::default_config();
        let h = streak_history("fast:7b", 2);
        let plan = plan_swap(&cfg, &upgrade_report(&cfg), &h, true, 3).expect("plan");
        assert!(!plan.eligible);
        assert_eq!(plan.consecutive_days, 2);
        assert!(plan.reasons.iter().any(|r| r.contains("2") && r.contains("3")));
    }

    #[test]
    fn opt_in_plus_streak_is_eligible() {
        let cfg = TrackerConfig::default_config();
        let h = streak_history("fast:7b", 3);
        let plan = plan_swap(&cfg, &upgrade_report(&cfg), &h, true, 3).expect("plan");
        assert!(plan.eligible);
        assert_eq!(plan.to, "fast:7b");
        assert_eq!(plan.from, cfg.baseline.model);
    }

    #[test]
    fn apply_swap_rejects_ineligible_plan() {
        let cfg = TrackerConfig::default_config();
        let h = streak_history("fast:7b", 1);
        let plan = plan_swap(&cfg, &upgrade_report(&cfg), &h, true, 3).expect("plan");
        assert!(apply_swap(&cfg, &plan).is_err());
    }

    #[test]
    fn apply_swap_produces_valid_config_with_new_baseline() {
        let cfg = TrackerConfig::default_config();
        let h = streak_history("fast:7b", 4);
        let plan = plan_swap(&cfg, &upgrade_report(&cfg), &h, true, 3).expect("plan");
        let next = apply_swap(&cfg, &plan).expect("apply");
        assert_eq!(next.baseline.model, "fast:7b");
        assert_eq!(next.baseline.vram_gib, 8.0);
        // Phase 0 mandate survives: the new config never auto-swaps.
        assert!(!next.selection.auto_swap);
        assert!(next.validate().is_ok());
    }
}

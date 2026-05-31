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

use serde::{Deserialize, Serialize};

use crate::config::TrackerConfig;
use crate::error::{TrackerError, TrackerResult};
use crate::report::DailyReport;
use crate::selection::SelectionStatus;
use crate::trend::TrendHistory;

/// The outcome of evaluating a swap. Always rendered to the operator;
/// `eligible` is the only field that gates [`apply_swap`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapPlan {
    pub from: String,
    pub to: String,
    /// Candidate footprint carried from the poll row so [`apply_swap`]
    /// can stamp the new baseline correctly.
    pub to_vram_gib: f32,
    pub to_disk_gib: f32,
    pub to_quant: String,
    pub eligible: bool,
    pub consecutive_days: u32,
    pub required_days: u32,
    pub reasons: Vec<String>,
}

/// Pick the strongest UpgradeCandidate in today's report: highest
/// quality delta, throughput as the tiebreak.
fn best_upgrade<'a>(report: &'a DailyReport) -> Option<&'a crate::selection::Verdict> {
    report
        .verdicts
        .iter()
        .filter(|v| v.status == SelectionStatus::UpgradeCandidate)
        .max_by(|a, b| {
            a.quality_delta
                .partial_cmp(&b.quality_delta)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.throughput_uplift
                        .partial_cmp(&b.throughput_uplift)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
}

/// Decide whether the seat should swap to today's best candidate. Returns
/// `None` when there is nothing to consider. The plan is `eligible` only
/// when the operator opted in **and** the candidate cleared a floor on
/// `required_days` consecutive days.
pub fn plan_swap(
    cfg: &TrackerConfig,
    report: &DailyReport,
    history: &TrendHistory,
    opt_in: bool,
    required_days: u32,
) -> Option<SwapPlan> {
    let best = best_upgrade(report)?;
    let to = best.model_id.clone();

    let consecutive_days = history
        .model(&to)
        .map(|m| m.consecutive_upgrade_days())
        .unwrap_or(0);

    let footprint = report.poll.candidates.iter().find(|c| c.model_id == to);
    let to_vram_gib = footprint.map(|c| c.vram_gib).unwrap_or(cfg.baseline.vram_gib);
    let to_disk_gib = footprint.map(|c| c.disk_gib).unwrap_or(cfg.baseline.disk_gib);
    let to_quant = footprint
        .map(|c| c.quant.clone())
        .unwrap_or_else(|| cfg.baseline.quant.clone());

    let mut reasons = Vec::new();
    let streak_ok = consecutive_days >= required_days;
    if streak_ok {
        reasons.push(format!(
            "cleared a floor on {} consecutive days (>= required {})",
            consecutive_days, required_days
        ));
    } else {
        reasons.push(format!(
            "streak {} < required {} consecutive days",
            consecutive_days, required_days
        ));
    }
    if !opt_in {
        reasons.push(
            "Phase 1 opt-in required: pass --opt-in to apply (auto-swap stays off by default)"
                .to_string(),
        );
    }

    let eligible = opt_in && streak_ok;
    if eligible {
        reasons.push("operator opt-in confirmed → swap eligible".to_string());
    }

    Some(SwapPlan {
        from: cfg.baseline.model.clone(),
        to,
        to_vram_gib,
        to_disk_gib,
        to_quant,
        eligible,
        consecutive_days,
        required_days,
        reasons,
    })
}

/// Materialise an eligible plan into a new [`TrackerConfig`]. Errors when
/// the plan is not eligible. The returned config keeps `auto_swap = false`
/// so it passes `validate()` and the daily cron stays report-only.
pub fn apply_swap(cfg: &TrackerConfig, plan: &SwapPlan) -> TrackerResult<TrackerConfig> {
    if !plan.eligible {
        return Err(TrackerError::Config(format!(
            "swap to `{}` is not eligible: {}",
            plan.to,
            plan.reasons.join("; ")
        )));
    }
    let mut next = cfg.clone();
    next.baseline.model = plan.to.clone();
    next.baseline.vram_gib = plan.to_vram_gib;
    next.baseline.disk_gib = plan.to_disk_gib;
    next.baseline.quant = plan.to_quant.clone();
    // Explicit operator decision — never flip auto_swap on.
    next.selection.auto_swap = false;
    next.validate()?;
    Ok(next)
}

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

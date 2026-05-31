// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Notification builder — turns a finished [`DailyReport`] into an
//! optional human-facing notice when the daily run surfaces an
//! actionable upgrade candidate.
//!
//! Phase 0 stays report-only, so the notifier never *acts* — it only
//! renders a payload that the LaunchAgent wrapper can hand to
//! `terminal-notifier` (macOS) or post to a webhook. When no candidate
//! clears a floor, [`build_notification`] returns `None` so a silent
//! day produces no noise.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{cave_prompts, synth_snapshot, BenchSnapshot, EvalResult};
    use crate::config::TrackerConfig;
    use crate::poll::PollSummary;
    use crate::report::DailyReport;
    use crate::selection::{baseline_verdict, evaluate};
    use crate::registry::{Candidate, SourceKind};

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

    /// Build a report whose single candidate is a live UpgradeCandidate
    /// (cleared the speed floor by a wide margin).
    fn report_with_upgrade() -> DailyReport {
        let cfg = TrackerConfig::default_config();
        let base = snap(&cfg.baseline.model, 0.5, 400, 1000);
        let c = cand("fast:7b");
        let cb = snap(&c.model_id, 0.5, 1200, 1000);
        let v = evaluate(&cfg, &c, &cb, &base);
        let poll = PollSummary::from_seed_only();
        DailyReport::assemble(
            &cfg,
            poll,
            base,
            vec![cb],
            vec![baseline_verdict(&cfg.baseline.model), v],
        )
    }

    /// Build a report with no upgrade candidate (synth snapshots → Unknown).
    fn quiet_report() -> DailyReport {
        let cfg = TrackerConfig::default_config();
        let base = synth_snapshot(&cfg.baseline.model);
        let poll = PollSummary::from_seed_only();
        DailyReport::assemble(
            &cfg,
            poll,
            base,
            vec![],
            vec![baseline_verdict(&cfg.baseline.model)],
        )
    }

    #[test]
    fn quiet_day_produces_no_notification() {
        assert!(build_notification(&quiet_report()).is_none());
    }

    #[test]
    fn upgrade_day_fires_with_upgrade_severity() {
        let n = build_notification(&report_with_upgrade()).expect("notification");
        assert_eq!(n.severity, NoticeSeverity::Upgrade);
        assert!(n.candidate_models.iter().any(|m| m == "fast:7b"));
    }

    #[test]
    fn title_counts_candidates() {
        let n = build_notification(&report_with_upgrade()).expect("notification");
        assert!(n.title.contains('1'), "title should carry the count: {}", n.title);
        assert!(n.title.to_lowercase().contains("upgrade"));
    }

    #[test]
    fn body_names_model_and_baseline() {
        let n = build_notification(&report_with_upgrade()).expect("notification");
        assert!(n.body.contains("fast:7b"));
        assert!(n.body.contains("qwen3.6:35b-a3b-coding-mxfp8"));
    }

    #[test]
    fn render_text_is_title_then_body() {
        let n = build_notification(&report_with_upgrade()).expect("notification");
        let t = n.render_text();
        assert!(t.starts_with(&n.title));
        assert!(t.contains(&n.body));
    }
}

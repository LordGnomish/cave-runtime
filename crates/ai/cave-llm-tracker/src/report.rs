// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Report emitters — produce a sibling `.md` (human-readable digest)
//! and `.json` (machine record) for one daily run.
//!
//! The JSON wire layout is the same one the cave-portal admin page will
//! consume in Phase 1; keeping it stable lets the cron tick and the
//! portal evolve independently.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::bench::BenchSnapshot;
use crate::config::TrackerConfig;
use crate::error::{TrackerError, TrackerResult};
use crate::poll::PollSummary;
use crate::registry::Candidate;
use crate::selection::Verdict;

/// One full daily report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReport {
    pub schema_version: u32,
    pub generated_at_utc: String,
    pub baseline: String,
    pub config_summary: ConfigSummary,
    pub poll: PollSummary,
    pub baseline_bench: BenchSnapshot,
    pub candidate_benches: Vec<BenchSnapshot>,
    pub verdicts: Vec<Verdict>,
    /// Phase 0 mandate banner — repeated in JSON so downstream tooling
    /// cannot accidentally interpret a "would-swap" as an "did-swap".
    pub phase_0_no_auto_swap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSummary {
    pub baseline_model: String,
    pub speed_uplift_floor: f32,
    pub eval_uplift_floor: f32,
    pub max_vram_gib: f32,
    pub max_disk_gib: f32,
    pub license_allowlist: Vec<String>,
    pub sources_enabled: Vec<String>,
}

impl ConfigSummary {
    pub fn from_config(cfg: &TrackerConfig) -> Self {
        let mut sources = Vec::new();
        if cfg.sources.huggingface {
            sources.push("huggingface".to_string());
        }
        if cfg.sources.ollama_library {
            sources.push("ollama_library".to_string());
        }
        if cfg.sources.lmsys_leaderboard {
            sources.push("lmsys_leaderboard".to_string());
        }
        if cfg.sources.github_backend_releases {
            sources.push("github_backend".to_string());
        }
        Self {
            baseline_model: cfg.baseline.model.clone(),
            speed_uplift_floor: cfg.selection.speed_uplift_floor,
            eval_uplift_floor: cfg.selection.eval_uplift_floor,
            max_vram_gib: cfg.guards.max_vram_gib,
            max_disk_gib: cfg.guards.max_disk_gib,
            license_allowlist: cfg.guards.license_allowlist.clone(),
            sources_enabled: sources,
        }
    }
}

impl DailyReport {
    pub fn assemble(
        cfg: &TrackerConfig,
        poll: PollSummary,
        baseline_bench: BenchSnapshot,
        candidate_benches: Vec<BenchSnapshot>,
        verdicts: Vec<Verdict>,
    ) -> Self {
        Self {
            schema_version: 1,
            generated_at_utc: chrono::Utc::now().to_rfc3339(),
            baseline: cfg.baseline.model.clone(),
            config_summary: ConfigSummary::from_config(cfg),
            poll,
            baseline_bench,
            candidate_benches,
            verdicts,
            phase_0_no_auto_swap: !cfg.selection.auto_swap,
        }
    }

    pub fn to_json(&self) -> TrackerResult<String> {
        serde_json::to_string_pretty(self).map_err(TrackerError::from)
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!(
            "# cave-llm-tracker — daily report ({})\n\n",
            self.generated_at_utc
        ));
        md.push_str(&format!("- baseline: `{}`\n", self.baseline));
        md.push_str(&format!(
            "- candidates polled: {}\n",
            self.poll.candidates.len()
        ));
        md.push_str(&format!(
            "- phase 0 auto-swap: **disabled** ({})\n\n",
            self.phase_0_no_auto_swap
        ));

        md.push_str("## Per-source counts\n\n");
        let mut pairs: Vec<(&String, &usize)> = self.poll.per_source_count.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in pairs {
            md.push_str(&format!("- `{}`: {}\n", k, v));
        }
        md.push('\n');

        if !self.poll.source_errors.is_empty() {
            md.push_str("## Source errors\n\n");
            let mut errs: Vec<(&String, &String)> = self.poll.source_errors.iter().collect();
            errs.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in errs {
                md.push_str(&format!("- `{}`: {}\n", k, v));
            }
            md.push('\n');
        }

        md.push_str("## Verdicts\n\n");
        md.push_str("| Model | Status | Δ quality | Δ throughput | Reasons |\n");
        md.push_str("|-------|--------|-----------|--------------|---------|\n");
        for v in &self.verdicts {
            md.push_str(&format!(
                "| `{}` | {:?} | {:+.3} | {:+.2}% | {} |\n",
                v.model_id,
                v.status,
                v.quality_delta,
                v.throughput_uplift * 100.0,
                v.reasons.join("; ")
            ));
        }
        md
    }

    pub fn write_to_dir(&self, dir: &Path, date_stamp: &str) -> TrackerResult<(PathBuf, PathBuf)> {
        std::fs::create_dir_all(dir)?;
        let json_path = dir.join(format!("daily-{}.json", date_stamp));
        let md_path = dir.join(format!("daily-{}.md", date_stamp));
        std::fs::write(&json_path, self.to_json()?)?;
        std::fs::write(&md_path, self.to_markdown())?;
        Ok((json_path, md_path))
    }
}

/// Defensive scan over the poll summary — returns the candidates that
/// should be auto-graded against the baseline this run (skip the
/// baseline itself; cap to N for live bench, since each prompt costs
/// real seconds against Ollama).
pub fn shortlist<'a>(poll: &'a PollSummary, baseline: &str, cap: usize) -> Vec<&'a Candidate> {
    poll.candidates
        .iter()
        .filter(|c| c.model_id != baseline)
        .take(cap)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::synth_snapshot;
    use crate::poll::PollSummary;
    use crate::selection::baseline_verdict;

    fn empty_report() -> DailyReport {
        let cfg = TrackerConfig::default_config();
        let poll = PollSummary::from_seed_only();
        let base = synth_snapshot(&cfg.baseline.model);
        DailyReport::assemble(
            &cfg,
            poll,
            base.clone(),
            vec![],
            vec![baseline_verdict(&cfg.baseline.model)],
        )
    }

    #[test]
    fn assemble_records_phase_0_no_auto_swap_true() {
        let r = empty_report();
        assert!(r.phase_0_no_auto_swap);
    }

    #[test]
    fn json_serialises_with_pretty_indentation() {
        let r = empty_report();
        let j = r.to_json().unwrap();
        assert!(j.contains("\n  \"schema_version\""));
    }

    #[test]
    fn markdown_lists_baseline_and_verdicts_header() {
        let r = empty_report();
        let md = r.to_markdown();
        assert!(md.contains("# cave-llm-tracker"));
        assert!(md.contains("baseline:"));
        assert!(md.contains("Verdicts"));
    }

    #[test]
    fn shortlist_skips_baseline_and_caps() {
        let cfg = TrackerConfig::default_config();
        let poll = PollSummary::from_seed_only();
        let list = shortlist(&poll, &cfg.baseline.model, 3);
        assert!(list.len() <= 3);
        assert!(list.iter().all(|c| c.model_id != cfg.baseline.model));
    }

    #[test]
    fn write_to_dir_emits_both_md_and_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let r = empty_report();
        let (jp, mp) = r.write_to_dir(dir.path(), "2026-05-21").expect("write");
        assert!(jp.exists() && mp.exists());
        let j = std::fs::read_to_string(&jp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed["schema_version"], 1);
    }
}

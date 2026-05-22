// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Selection — compare a baseline `BenchSnapshot` against each candidate
//! and label whether the candidate clears the speed-uplift or quality-
//! uplift thresholds (Phase 0: never auto-swap, only flag).
//!
//! Constraints applied first:
//!   * License must be in the allowlist.
//!   * VRAM / disk must be under guard ceilings.
//!   * Anything failing these is `Rejected` regardless of bench scores.

use serde::{Deserialize, Serialize};

use crate::bench::BenchSnapshot;
use crate::config::TrackerConfig;
use crate::registry::Candidate;

/// Per-candidate decision rendered into the daily report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Verdict {
    pub model_id: String,
    pub status: SelectionStatus,
    pub reasons: Vec<String>,
    /// `cand.score - baseline.score` (quality, 0.0–1.0).
    pub quality_delta: f32,
    /// `(cand.tput - baseline.tput) / baseline.tput` — fractional uplift.
    /// `0.0` if either side has no measurement.
    pub throughput_uplift: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStatus {
    /// Baseline row — never an upgrade candidate, always emitted for
    /// reference.
    Baseline,
    /// Cleared every guard and at least one uplift floor — would be
    /// auto-swapped in Phase 1+. Phase 0 only flags.
    UpgradeCandidate,
    /// Cleared guards but failed both uplift floors.
    Below,
    /// Failed a hard guard (license / VRAM / disk).
    Rejected,
    /// Bench was a no-op (e.g. seed-only report). Cannot decide.
    Unknown,
}

/// Apply the guard rails + uplift floors to one candidate, given the
/// baseline snapshot. Always returns a verdict; never panics.
pub fn evaluate(
    cfg: &TrackerConfig,
    cand: &Candidate,
    cand_bench: &BenchSnapshot,
    baseline_bench: &BenchSnapshot,
) -> Verdict {
    let mut reasons: Vec<String> = Vec::new();
    let mut rejected = false;

    if !cfg
        .guards
        .license_allowlist
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&cand.license))
    {
        reasons.push(format!(
            "license `{}` not in allowlist {:?}",
            cand.license, cfg.guards.license_allowlist
        ));
        rejected = true;
    }
    if cand.vram_gib > cfg.guards.max_vram_gib {
        reasons.push(format!(
            "VRAM {:.1} GiB exceeds guard {:.1} GiB",
            cand.vram_gib, cfg.guards.max_vram_gib
        ));
        rejected = true;
    }
    if cand.disk_gib > cfg.guards.max_disk_gib {
        reasons.push(format!(
            "disk {:.1} GiB exceeds guard {:.1} GiB",
            cand.disk_gib, cfg.guards.max_disk_gib
        ));
        rejected = true;
    }

    let baseline_quality = baseline_bench.mean_quality();
    let cand_quality = cand_bench.mean_quality();
    let quality_delta = cand_quality - baseline_quality;

    let baseline_tput = baseline_bench.throughput_bytes_per_sec();
    let cand_tput = cand_bench.throughput_bytes_per_sec();
    let throughput_uplift = if baseline_tput > 0.0 && cand_tput > 0.0 {
        (cand_tput - baseline_tput) / baseline_tput
    } else {
        0.0
    };

    if rejected {
        return Verdict {
            model_id: cand.model_id.clone(),
            status: SelectionStatus::Rejected,
            reasons,
            quality_delta,
            throughput_uplift,
        };
    }

    // Phase 0 mandate is enforced in config::validate; we re-assert here
    // to make the audit trail explicit in the verdict reasons.
    if cfg.selection.auto_swap {
        reasons.push("PHASE_0_GUARD: auto_swap was true; report-only mode forced".to_string());
    }

    let no_bench_data = baseline_bench.results.iter().all(|r| r.timed_out)
        || cand_bench.results.iter().all(|r| r.timed_out);
    if no_bench_data {
        reasons.push("no live bench results; selection deferred".to_string());
        return Verdict {
            model_id: cand.model_id.clone(),
            status: SelectionStatus::Unknown,
            reasons,
            quality_delta,
            throughput_uplift,
        };
    }

    let speed_ok = throughput_uplift >= cfg.selection.speed_uplift_floor;
    let quality_ok = quality_delta
        >= cfg.selection.eval_uplift_floor.max(0.0);

    if speed_ok {
        reasons.push(format!(
            "throughput uplift {:.2}% clears speed floor {:.2}%",
            throughput_uplift * 100.0,
            cfg.selection.speed_uplift_floor * 100.0
        ));
    }
    if quality_ok {
        reasons.push(format!(
            "quality delta +{:.3} clears eval floor +{:.3}",
            quality_delta, cfg.selection.eval_uplift_floor
        ));
    }

    let status = if speed_ok || quality_ok {
        SelectionStatus::UpgradeCandidate
    } else {
        reasons.push(format!(
            "no floor cleared (throughput {:.2}%, quality Δ {:+.3})",
            throughput_uplift * 100.0,
            quality_delta
        ));
        SelectionStatus::Below
    };
    Verdict {
        model_id: cand.model_id.clone(),
        status,
        reasons,
        quality_delta,
        throughput_uplift,
    }
}

/// Convenience wrapper — produce a [`Verdict`] for the baseline itself
/// so the report shows it on the same row layout.
pub fn baseline_verdict(model_id: &str) -> Verdict {
    Verdict {
        model_id: model_id.to_string(),
        status: SelectionStatus::Baseline,
        reasons: vec!["current baseline (Burak seat)".to_string()],
        quality_delta: 0.0,
        throughput_uplift: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{cave_prompts, BenchSnapshot, EvalResult};
    use crate::registry::{Candidate, SourceKind};

    fn snap(model: &str, quality: f32, bytes_per_prompt: usize, ms_per_prompt: u64) -> BenchSnapshot {
        BenchSnapshot {
            model_id: model.to_string(),
            results: cave_prompts()
                .into_iter()
                .map(|p| EvalResult {
                    prompt_id: p.id.to_string(),
                    category: p.category.to_string(),
                    model_id: model.to_string(),
                    elapsed_ms: ms_per_prompt,
                    response_bytes: bytes_per_prompt,
                    quality,
                    timed_out: false,
                })
                .collect(),
        }
    }

    fn cand(model: &str, license: &str, vram: f32, disk: f32) -> Candidate {
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: model.to_string(),
            family: model.to_string(),
            license: license.to_string(),
            vram_gib: vram,
            disk_gib: disk,
            quant: "Q4_K_M".to_string(),
            upstream_ref: model.to_string(),
            score_hint: None,
        }
    }

    #[test]
    fn rejected_when_license_not_in_allowlist() {
        let cfg = TrackerConfig::default_config();
        let c = cand("foo:7b", "Proprietary", 8.0, 9.0);
        let v = evaluate(&cfg, &c, &snap(&c.model_id, 0.9, 600, 1000), &snap("base", 0.5, 400, 1000));
        assert_eq!(v.status, SelectionStatus::Rejected);
        assert!(v.reasons.iter().any(|r| r.contains("license")));
    }

    #[test]
    fn rejected_when_vram_exceeds_guard() {
        let cfg = TrackerConfig::default_config();
        let c = cand("huge:200b", "Apache-2.0", 200.0, 220.0);
        let v = evaluate(&cfg, &c, &snap(&c.model_id, 0.9, 600, 1000), &snap("base", 0.5, 400, 1000));
        assert_eq!(v.status, SelectionStatus::Rejected);
        assert!(v.reasons.iter().any(|r| r.contains("VRAM")));
    }

    #[test]
    fn upgrade_candidate_when_speed_floor_cleared() {
        let cfg = TrackerConfig::default_config();
        let c = cand("fast:7b", "MIT", 8.0, 9.0);
        // baseline ~400 B/s, candidate ~1200 B/s → 200% uplift (>>10%).
        let base = snap("base", 0.5, 400, 1000);
        let cand_bench = snap(&c.model_id, 0.5, 1200, 1000);
        let v = evaluate(&cfg, &c, &cand_bench, &base);
        assert_eq!(v.status, SelectionStatus::UpgradeCandidate);
        assert!(v.throughput_uplift >= 0.10);
    }

    #[test]
    fn upgrade_candidate_when_quality_floor_cleared() {
        let cfg = TrackerConfig::default_config();
        let c = cand("smart:7b", "Apache-2.0", 8.0, 9.0);
        let base = snap("base", 0.50, 400, 1000);
        let cand_bench = snap(&c.model_id, 0.70, 400, 1000);
        let v = evaluate(&cfg, &c, &cand_bench, &base);
        assert_eq!(v.status, SelectionStatus::UpgradeCandidate);
        assert!(v.quality_delta >= 0.05);
    }

    #[test]
    fn below_when_neither_floor_cleared() {
        let cfg = TrackerConfig::default_config();
        let c = cand("meh:7b", "Apache-2.0", 8.0, 9.0);
        let base = snap("base", 0.50, 400, 1000);
        let cand_bench = snap(&c.model_id, 0.51, 405, 1000);
        let v = evaluate(&cfg, &c, &cand_bench, &base);
        assert_eq!(v.status, SelectionStatus::Below);
    }

    #[test]
    fn unknown_when_no_live_bench_data() {
        let cfg = TrackerConfig::default_config();
        let c = cand("noop:7b", "MIT", 8.0, 9.0);
        let synth = crate::bench::synth_snapshot(&c.model_id);
        let base_synth = crate::bench::synth_snapshot("base");
        let v = evaluate(&cfg, &c, &synth, &base_synth);
        assert_eq!(v.status, SelectionStatus::Unknown);
    }

    #[test]
    fn baseline_verdict_is_baseline() {
        let v = baseline_verdict("qwen3.6:35b-a3b-coding-mxfp8");
        assert_eq!(v.status, SelectionStatus::Baseline);
        assert!(v.reasons.iter().any(|r| r.contains("baseline")));
    }
}

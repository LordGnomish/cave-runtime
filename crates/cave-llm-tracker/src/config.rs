// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TOML configuration for the daily tracker.
//!
//! The default config encodes Burak's current local-LLM seat
//! (`qwen3.6:35b-a3b-coding-mxfp8`) as the baseline against which
//! every candidate is compared, plus VRAM/disk guards and license
//! allow-list. Override fields by writing
//! `~/Library/Application Support/cave-runtime/llm-tracker/config.toml`
//! and pointing the CLI at it via `--config`.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{TrackerError, TrackerResult};

/// Top-level config consumed by the CLI and the daily LaunchAgent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackerConfig {
    pub baseline: BaselineConfig,
    pub sources: SourcesConfig,
    pub guards: GuardsConfig,
    pub selection: SelectionConfig,
    pub bench: BenchConfig,
    pub report: ReportConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineConfig {
    /// Model id Ollama recognises — `family:tag`.
    pub model: String,
    /// Approximate VRAM footprint in GiB at the configured quant.
    pub vram_gib: f32,
    /// Approximate on-disk footprint in GiB.
    pub disk_gib: f32,
    /// Quant scheme (informational; e.g. `mxfp8`, `Q4_K_M`).
    pub quant: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourcesConfig {
    pub huggingface: bool,
    pub ollama_library: bool,
    pub lmsys_leaderboard: bool,
    pub github_backend_releases: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuardsConfig {
    /// Hard ceiling for VRAM consumption (GiB).
    pub max_vram_gib: f32,
    /// Hard ceiling for on-disk model size (GiB).
    pub max_disk_gib: f32,
    /// SPDX identifiers that are acceptable; anything else is rejected.
    pub license_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectionConfig {
    /// Fractional speed improvement (median tokens/sec) required to flag
    /// a candidate for upgrade on speed alone.
    pub speed_uplift_floor: f32,
    /// Fractional eval-score improvement required to flag a candidate for
    /// upgrade on quality alone.
    pub eval_uplift_floor: f32,
    /// Phase 0 mandate: never swap baseline automatically. Phase 1+ may
    /// flip this, but for now it stays hard-wired off.
    pub auto_swap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchConfig {
    /// Wall-clock budget per prompt in seconds. Above this the candidate
    /// is marked `timed_out` instead of producing a score.
    pub per_prompt_timeout_secs: u32,
    /// Ollama HTTP endpoint used for live bench runs. The CLI's
    /// `--mode bench` requires Ollama to be reachable here; `--mode
    /// report` falls back to "no live bench" rows when unreachable.
    pub ollama_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportConfig {
    /// Output directory where daily reports land.
    pub output_dir: String,
    /// If true, also write a `latest.json` symlink-compatible copy.
    pub emit_latest: bool,
}

impl TrackerConfig {
    /// Burak's stamped baseline (qwen3.6:35b-a3b-coding-mxfp8) and the
    /// always-latest source set.
    pub fn default_config() -> Self {
        Self {
            baseline: BaselineConfig {
                model: "qwen3.6:35b-a3b-coding-mxfp8".to_string(),
                vram_gib: 22.0,
                disk_gib: 24.0,
                quant: "mxfp8".to_string(),
            },
            sources: SourcesConfig {
                huggingface: true,
                ollama_library: true,
                lmsys_leaderboard: true,
                github_backend_releases: true,
            },
            guards: GuardsConfig {
                max_vram_gib: 64.0,
                max_disk_gib: 96.0,
                license_allowlist: vec![
                    "Apache-2.0".to_string(),
                    "MIT".to_string(),
                    "AGPL-3.0-or-later".to_string(),
                    "AGPL-3.0".to_string(),
                ],
            },
            selection: SelectionConfig {
                speed_uplift_floor: 0.10,
                eval_uplift_floor: 0.05,
                // Phase 0 mandate — never auto-swap.
                auto_swap: false,
            },
            bench: BenchConfig {
                per_prompt_timeout_secs: 90,
                ollama_endpoint: "http://127.0.0.1:11434".to_string(),
            },
            report: ReportConfig {
                output_dir: "~/Library/Application Support/cave-runtime/llm-tracker".to_string(),
                emit_latest: true,
            },
        }
    }

    pub fn load(path: &Path) -> TrackerResult<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: TrackerConfig = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> TrackerResult<()> {
        if self.selection.auto_swap {
            return Err(TrackerError::Config(
                "selection.auto_swap MUST be false in Phase 0 — Burak mandate"
                    .to_string(),
            ));
        }
        if self.guards.license_allowlist.is_empty() {
            return Err(TrackerError::Config(
                "guards.license_allowlist must not be empty".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&self.selection.speed_uplift_floor) {
            return Err(TrackerError::Config(
                "selection.speed_uplift_floor must be a fraction in [0,1]".to_string(),
            ));
        }
        if !(0.0..=1.0).contains(&self.selection.eval_uplift_floor) {
            return Err(TrackerError::Config(
                "selection.eval_uplift_floor must be a fraction in [0,1]".to_string(),
            ));
        }
        Ok(())
    }

    /// Expand a leading `~` in `report.output_dir` using `$HOME`. Returns
    /// the path unchanged on a non-Unix host or when `$HOME` is unset.
    pub fn expanded_output_dir(&self) -> String {
        Self::expand_tilde(&self.report.output_dir, std::env::var("HOME").ok().as_deref())
    }

    /// Pure-function variant of [`expanded_output_dir`] — explicit
    /// `$HOME` makes the behaviour unit-testable without mutating the
    /// process environment (Rust 2024 makes `set_var` `unsafe`).
    pub fn expand_tilde(raw: &str, home: Option<&str>) -> String {
        if let Some(rest) = raw.strip_prefix("~/")
            && let Some(home) = home
        {
            return format!("{}/{}", home.trim_end_matches('/'), rest);
        }
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_baseline_is_burak_seat() {
        let c = TrackerConfig::default_config();
        assert_eq!(c.baseline.model, "qwen3.6:35b-a3b-coding-mxfp8");
        assert_eq!(c.baseline.quant, "mxfp8");
    }

    #[test]
    fn default_enables_all_four_sources() {
        let s = TrackerConfig::default_config().sources;
        assert!(s.huggingface && s.ollama_library && s.lmsys_leaderboard && s.github_backend_releases);
    }

    #[test]
    fn default_license_allowlist_covers_phase_0_set() {
        let g = TrackerConfig::default_config().guards;
        for lic in ["Apache-2.0", "MIT", "AGPL-3.0-or-later"] {
            assert!(
                g.license_allowlist.iter().any(|x| x == lic),
                "license {lic} missing from default allowlist"
            );
        }
    }

    #[test]
    fn auto_swap_forced_off() {
        let c = TrackerConfig::default_config();
        assert!(!c.selection.auto_swap);
        let mut bad = c.clone();
        bad.selection.auto_swap = true;
        assert!(bad.validate().is_err(), "auto_swap=true must fail validate()");
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let c = TrackerConfig::default_config();
        let s = toml::to_string_pretty(&c).expect("ser");
        let back: TrackerConfig = toml::from_str(&s).expect("de");
        assert_eq!(c, back);
    }

    #[test]
    fn expand_tilde_uses_supplied_home() {
        assert_eq!(
            TrackerConfig::expand_tilde("~/cave-tmp", Some("/tmp/fakehome")),
            "/tmp/fakehome/cave-tmp"
        );
        assert_eq!(
            TrackerConfig::expand_tilde("~/cave-tmp", Some("/tmp/fakehome/")),
            "/tmp/fakehome/cave-tmp"
        );
    }

    #[test]
    fn expand_tilde_passthrough_without_home() {
        assert_eq!(
            TrackerConfig::expand_tilde("~/cave-tmp", None),
            "~/cave-tmp"
        );
        assert_eq!(
            TrackerConfig::expand_tilde("/abs/path", Some("/tmp/home")),
            "/abs/path"
        );
    }
}

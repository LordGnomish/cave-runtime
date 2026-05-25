// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cave-specific eval bench — five prompts that probe the model on the
//! actual day-to-day work this seat does for the runtime: Charter v2
//! close paperwork, parity-manifest TOML, a Rust refactor, dual-language
//! (TR+EN) replies, and a Conventional Commits message.
//!
//! Live runs talk to a local Ollama endpoint (`/api/generate`); the
//! result includes wall-time and a deterministic, model-agnostic
//! quality score derived from keyword hits + length plateau. Quality
//! scoring is *not* an LLM-judge — we keep it deterministic so reports
//! are reproducible offline.

use serde::{Deserialize, Serialize};

use crate::config::TrackerConfig;
use crate::error::{TrackerError, TrackerResult};

/// One prompt in the cave-specific suite. Static-only fields; the type
/// is exposed for inspection but is not (de)serialised — `EvalResult`
/// carries the relevant data into the daily report.
#[derive(Debug, Clone)]
pub struct EvalPrompt {
    pub id: &'static str,
    pub category: &'static str,
    pub prompt: &'static str,
    /// Substrings whose presence in the response counts toward the
    /// quality score. Lowercase; ASCII compare.
    pub must_contain: &'static [&'static str],
}

/// The canonical 5-prompt suite. Order is stable so historic reports
/// can be diffed line-for-line.
pub fn cave_prompts() -> Vec<EvalPrompt> {
    vec![
        EvalPrompt {
            id: "charter_v2_close",
            category: "Charter v2 paperwork",
            prompt: "Draft the close-out checklist for a Charter v2 crate. Include all 8 gates by name (upstream version pinned, source_sha present, fill_ratio measured, parity_ratio_source manifest, last_audit date, mapped+partial+skipped+unmapped sum to total, no-stub macros in src, AGPL SPDX header on every .rs file). Output as a markdown checklist.",
            must_contain: &[
                "fill_ratio",
                "source_sha",
                "parity_ratio_source",
                "spdx",
                "agpl",
                "skipped",
            ],
        },
        EvalPrompt {
            id: "parity_manifest_toml",
            category: "Parity manifest TOML",
            prompt: "Produce a parity.manifest.toml [upstream] block for a hypothetical cave-foo crate that ports Apache Foo v1.2.3 (commit abc1234, Apache-2.0, Java). Then produce a [parity] block with parity_ratio_source = \"manifest\", fill_ratio = 0.96, honest_ratio = 0.72, last_audit = 2026-05-21, mapped_count = 18, partial_count = 1, skipped_count = 6, unmapped_count = 0, total = 25. TOML only, no commentary.",
            must_contain: &[
                "[upstream]",
                "[parity]",
                "fill_ratio",
                "apache-2.0",
                "source_sha",
            ],
        },
        EvalPrompt {
            id: "rust_refactor",
            category: "Rust refactor",
            prompt: "Refactor this Rust function to use `?` and remove unwrap: `fn read_id(p: &str) -> u64 { let s = std::fs::read_to_string(p).unwrap(); s.trim().parse::<u64>().unwrap() }`. Return only the refactored function with a sensible error type.",
            must_contain: &["fn read_id", "?", "result", "parse"],
        },
        EvalPrompt {
            id: "tr_en_dual",
            category: "Dual-language (TR + EN)",
            prompt: "Explain in TWO short paragraphs — first in Turkish, then in English — what a Charter v2 8-gate close-out audit is and why fill_ratio above 0.95 matters for OSS launch credibility. Mark the sections with `## TR` and `## EN`.",
            must_contain: &["## tr", "## en", "fill_ratio", "charter"],
        },
        EvalPrompt {
            id: "conv_commit",
            category: "Conventional commit",
            prompt: "Write a Conventional Commits message for the following diff summary: a new src/registry.rs module with HuggingFace + Ollama + LMSys + GitHub clients (~250 LOC) plus tests; ports nothing — pure original work. Use prefix `feat(cave-llm-tracker):` and include a short body. One commit message, no extra commentary.",
            must_contain: &[
                "feat(cave-llm-tracker):",
                "registry",
                "huggingface",
                "ollama",
            ],
        },
    ]
}

/// Outcome of evaluating one candidate against one prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub prompt_id: String,
    pub category: String,
    pub model_id: String,
    pub elapsed_ms: u64,
    /// Total bytes in the response (rough proxy for output length).
    pub response_bytes: usize,
    /// 0.0–1.0; fraction of `must_contain` substrings present in the
    /// (lowercased) response, scaled by a length plateau so over-long
    /// rambles do not score higher than focused answers.
    pub quality: f32,
    /// Set when the candidate breached `bench.per_prompt_timeout_secs`
    /// or the upstream API errored. The result is still emitted for
    /// audit; quality is `0.0` and `response_bytes` is `0`.
    pub timed_out: bool,
}

/// Aggregate of `EvalResult`s for one candidate — what selection compares.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSnapshot {
    pub model_id: String,
    pub results: Vec<EvalResult>,
}

impl BenchSnapshot {
    /// Mean quality across all `results`. `0.0` for empty snapshots.
    pub fn mean_quality(&self) -> f32 {
        if self.results.is_empty() {
            return 0.0;
        }
        let s: f32 = self.results.iter().map(|r| r.quality).sum();
        s / self.results.len() as f32
    }

    /// Total elapsed across the suite (ms); never panics on empty.
    pub fn total_elapsed_ms(&self) -> u64 {
        self.results.iter().map(|r| r.elapsed_ms).sum()
    }

    /// Aggregate tokens-per-second proxy: `total_bytes / total_seconds`.
    /// Bytes are a tolerable stand-in for tokens; the *ratio* between
    /// two candidates is what selection uses, not the absolute number.
    pub fn throughput_bytes_per_sec(&self) -> f32 {
        let total_bytes: usize = self.results.iter().map(|r| r.response_bytes).sum();
        let total_secs = self.total_elapsed_ms() as f32 / 1000.0;
        if total_secs <= 0.0 {
            0.0
        } else {
            total_bytes as f32 / total_secs
        }
    }
}

/// Deterministic quality scorer used by the live `score_response` and
/// directly by unit tests. Exposed so other crates (cave-portal,
/// cave-obs) can render identical figures.
pub fn score_response(prompt: &EvalPrompt, response: &str) -> f32 {
    let lower = response.to_lowercase();
    if lower.is_empty() {
        return 0.0;
    }
    let hits = prompt
        .must_contain
        .iter()
        .filter(|kw| lower.contains(&kw.to_lowercase()))
        .count();
    let keyword_score = if prompt.must_contain.is_empty() {
        1.0
    } else {
        hits as f32 / prompt.must_contain.len() as f32
    };
    // Length plateau: full credit at >= 200 bytes, linear up to that,
    // then a soft penalty above 4000 bytes for rambling answers.
    let bytes = response.len() as f32;
    let length_score = if bytes <= 200.0 {
        bytes / 200.0
    } else if bytes <= 4000.0 {
        1.0
    } else {
        (8000.0 - bytes.min(8000.0)) / 4000.0
    }
    .clamp(0.0, 1.0);
    (0.7 * keyword_score + 0.3 * length_score).clamp(0.0, 1.0)
}

/// Live run against a local Ollama-compatible endpoint. Each prompt is
/// issued via POST `/api/generate` with `stream:false`. Transport or
/// upstream errors mark the row `timed_out` instead of aborting — the
/// whole report is best-effort by design.
pub async fn run_bench(
    cfg: &TrackerConfig,
    model_id: &str,
) -> TrackerResult<BenchSnapshot> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            cfg.bench.per_prompt_timeout_secs as u64,
        ))
        .build()
        .map_err(|e| TrackerError::Bench(format!("client: {e}")))?;
    let mut results = Vec::new();
    for p in cave_prompts() {
        let body = serde_json::json!({
            "model": model_id,
            "prompt": p.prompt,
            "stream": false,
        });
        let started = std::time::Instant::now();
        let url = format!("{}/api/generate", cfg.bench.ollama_endpoint.trim_end_matches('/'));
        let response_text = match client.post(&url).json(&body).send().await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(j) => j
                    .get("response")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .unwrap_or_default(),
                Err(_) => String::new(),
            },
            Err(_) => String::new(),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let timed_out = response_text.is_empty();
        let quality = if timed_out { 0.0 } else { score_response(&p, &response_text) };
        results.push(EvalResult {
            prompt_id: p.id.to_string(),
            category: p.category.to_string(),
            model_id: model_id.to_string(),
            elapsed_ms,
            response_bytes: response_text.len(),
            quality,
            timed_out,
        });
    }
    Ok(BenchSnapshot {
        model_id: model_id.to_string(),
        results,
    })
}

/// Synthesise a `BenchSnapshot` from precomputed results — useful for
/// `--mode report` runs that skip the live bench entirely.
pub fn synth_snapshot(model_id: &str) -> BenchSnapshot {
    BenchSnapshot {
        model_id: model_id.to_string(),
        results: cave_prompts()
            .into_iter()
            .map(|p| EvalResult {
                prompt_id: p.id.to_string(),
                category: p.category.to_string(),
                model_id: model_id.to_string(),
                elapsed_ms: 0,
                response_bytes: 0,
                quality: 0.0,
                timed_out: true,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_has_exactly_five_prompts_with_unique_ids() {
        let prompts = cave_prompts();
        assert_eq!(prompts.len(), 5, "cave bench is fixed at five prompts");
        let mut ids: Vec<&str> = prompts.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5, "prompt ids must be unique");
    }

    #[test]
    fn five_categories_cover_charter_parity_refactor_dual_commit() {
        let cats: Vec<&str> = cave_prompts().iter().map(|p| p.category).collect();
        for must in [
            "Charter v2 paperwork",
            "Parity manifest TOML",
            "Rust refactor",
            "Dual-language (TR + EN)",
            "Conventional commit",
        ] {
            assert!(cats.contains(&must), "missing category: {must}");
        }
    }

    #[test]
    fn score_empty_response_is_zero() {
        let p = &cave_prompts()[0];
        assert_eq!(score_response(p, ""), 0.0);
    }

    #[test]
    fn score_full_match_with_decent_length_is_near_one() {
        let p = EvalPrompt {
            id: "synthetic",
            category: "synthetic",
            prompt: "",
            must_contain: &["alpha", "beta"],
        };
        let response = "alpha ".repeat(60) + "beta ";
        let score = score_response(&p, &response);
        assert!(
            score >= 0.9,
            "expected near-1 quality for full match + adequate length; got {score}"
        );
    }

    #[test]
    fn score_punishes_overlong_ramble() {
        let p = EvalPrompt {
            id: "synthetic",
            category: "synthetic",
            prompt: "",
            must_contain: &["alpha"],
        };
        let short = "alpha ".repeat(40);
        let ramble = "alpha ".repeat(2000);
        assert!(
            score_response(&p, &short) > score_response(&p, &ramble),
            "ramble must score lower than focused answer"
        );
    }

    #[test]
    fn synth_snapshot_marks_every_row_timed_out() {
        let s = synth_snapshot("any-model");
        assert_eq!(s.results.len(), 5);
        assert!(s.results.iter().all(|r| r.timed_out));
        assert_eq!(s.mean_quality(), 0.0);
        assert_eq!(s.throughput_bytes_per_sec(), 0.0);
    }
}

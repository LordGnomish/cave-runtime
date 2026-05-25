// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Aggregate poller — fans out across all enabled sources, then merges
//! and dedupes the returned candidates by `model_id`.
//!
//! Live sources are best-effort: a transport failure on any one source
//! is recorded in [`PollSummary::source_errors`] but never blocks the
//! aggregate result. The seed catalog is always present, so a totally
//! offline run still yields >= 5 candidates.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::TrackerConfig;
use crate::error::TrackerResult;
use crate::registry::{
    default_backend_repos, seed_catalog, Candidate, LiveFetcher, SourceKind,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollSummary {
    pub polled_at_utc: String,
    pub candidates: Vec<Candidate>,
    pub per_source_count: HashMap<String, usize>,
    pub source_errors: HashMap<String, String>,
}

impl PollSummary {
    pub fn total(&self) -> usize {
        self.candidates.len()
    }

    pub fn from_seed_only() -> Self {
        let seed = seed_catalog();
        let mut per_source = HashMap::new();
        per_source.insert(SourceKind::SeedCatalog.slug().to_string(), seed.len());
        Self {
            polled_at_utc: chrono::Utc::now().to_rfc3339(),
            candidates: seed,
            per_source_count: per_source,
            source_errors: HashMap::new(),
        }
    }
}

/// Run all enabled sources sequentially (the call volume is tiny and
/// the host's I/O is the bottleneck, not CPU). Returns a [`PollSummary`].
pub async fn poll_all(cfg: &TrackerConfig) -> TrackerResult<PollSummary> {
    let fetcher = LiveFetcher::new();
    let mut per_source: HashMap<String, usize> = HashMap::new();
    let mut errors: HashMap<String, String> = HashMap::new();
    let mut all: Vec<Candidate> = Vec::new();

    // Seed catalog first — guarantees a floor of >= 5 rows.
    let seed = seed_catalog();
    per_source.insert(SourceKind::SeedCatalog.slug().to_string(), seed.len());
    all.extend(seed);

    if cfg.sources.huggingface {
        match fetcher.fetch_huggingface(20).await {
            Ok(c) => {
                per_source.insert(SourceKind::HuggingFace.slug().to_string(), c.len());
                all.extend(c);
            }
            Err(e) => {
                errors.insert(SourceKind::HuggingFace.slug().to_string(), e.to_string());
            }
        }
    }

    if cfg.sources.ollama_library {
        match fetcher.fetch_ollama_library().await {
            Ok(c) => {
                per_source.insert(SourceKind::OllamaLibrary.slug().to_string(), c.len());
                all.extend(c);
            }
            Err(e) => {
                errors.insert(SourceKind::OllamaLibrary.slug().to_string(), e.to_string());
            }
        }
    }

    if cfg.sources.lmsys_leaderboard {
        match fetcher.fetch_lmsys().await {
            Ok(c) => {
                per_source.insert(SourceKind::LmsysLeaderboard.slug().to_string(), c.len());
                all.extend(c);
            }
            Err(e) => {
                errors.insert(SourceKind::LmsysLeaderboard.slug().to_string(), e.to_string());
            }
        }
    }

    if cfg.sources.github_backend_releases {
        match fetcher.fetch_github_backend(&default_backend_repos()).await {
            Ok(c) => {
                per_source.insert(SourceKind::GithubBackend.slug().to_string(), c.len());
                all.extend(c);
            }
            Err(e) => {
                errors.insert(SourceKind::GithubBackend.slug().to_string(), e.to_string());
            }
        }
    }

    let candidates = dedupe(all);
    Ok(PollSummary {
        polled_at_utc: chrono::Utc::now().to_rfc3339(),
        candidates,
        per_source_count: per_source,
        source_errors: errors,
    })
}

/// Dedupe by `model_id`, preferring the entry with the higher
/// `score_hint`, then by source priority (seed > LMSys > HF > Ollama > GH).
/// This keeps the seed catalog's authoritative VRAM/disk/license fields
/// when they exist.
pub fn dedupe(mut input: Vec<Candidate>) -> Vec<Candidate> {
    fn source_priority(s: SourceKind) -> u8 {
        match s {
            SourceKind::SeedCatalog => 0,
            SourceKind::LmsysLeaderboard => 1,
            SourceKind::HuggingFace => 2,
            SourceKind::OllamaLibrary => 3,
            SourceKind::GithubBackend => 4,
        }
    }
    input.sort_by(|a, b| {
        a.model_id.cmp(&b.model_id).then_with(|| {
            let sa = a.score_hint.unwrap_or(0.0);
            let sb = b.score_hint.unwrap_or(0.0);
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| source_priority(a.source).cmp(&source_priority(b.source)))
        })
    });
    input.dedup_by(|a, b| a.model_id == b.model_id);
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::seed_catalog;

    #[test]
    fn seed_only_summary_has_floor_candidates() {
        let s = PollSummary::from_seed_only();
        assert!(s.total() >= 5);
        assert!(!s.polled_at_utc.is_empty());
        assert!(s.source_errors.is_empty());
    }

    #[test]
    fn dedupe_collapses_same_model_id_and_prefers_higher_score() {
        let mut input = seed_catalog();
        let baseline = input[0].clone();
        // Add a duplicate of baseline as if from LMSys with a higher score.
        let mut dup = baseline.clone();
        dup.source = SourceKind::LmsysLeaderboard;
        dup.score_hint = Some(9999.0);
        input.push(dup);
        let out = dedupe(input);
        let kept = out
            .iter()
            .find(|c| c.model_id == baseline.model_id)
            .expect("baseline survived");
        assert_eq!(kept.score_hint, Some(9999.0));
    }
}

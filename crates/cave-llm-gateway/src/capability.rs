// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Capability-based router.
//!
//! Each registered model declares a [`ModelCapability`] (context window,
//! supports tools, supports vision, cost/token, etc.). A request describes
//! its [`CapabilityRequest`] (need_tools, need_vision, min_context,
//! preferred_locality, budget_cents_per_call). The router returns a
//! ranked list of models, highest score first.
//!
//! Scoring is deterministic and fully unit-testable — no randomness, no
//! provider RPCs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    /// Inference on-host (Ollama, llama.cpp, MLX).
    Local,
    /// Cloud SaaS (Anthropic, OpenAI, Mistral).
    Saas,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapability {
    pub provider: String,
    pub model: String,
    pub locality: Locality,
    pub context_window: u32,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_json: bool,
    /// Indicative cost per 1k input tokens, in micro-USD (10_000 = $0.01).
    pub cost_in_per_1k_micro_usd: u32,
    /// Indicative cost per 1k output tokens, in micro-USD.
    pub cost_out_per_1k_micro_usd: u32,
    /// Approximate quality score 0-100 (deterministic seed; refreshed by
    /// cave-llm-tracker daily reports).
    pub quality: u8,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityRequest {
    pub need_tools: bool,
    pub need_vision: bool,
    pub need_json: bool,
    pub min_context: u32,
    pub preferred_locality: Option<Locality>,
    pub max_cost_in_per_1k_micro_usd: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Scored<'a> {
    pub cap: &'a ModelCapability,
    pub score: f32,
}

/// Compute a single-model score against a request. Used as a building
/// block for [`CapabilityRouter::rank`].
pub fn score(req: &CapabilityRequest, cap: &ModelCapability) -> f32 {
    // Hard exclusions (mandatory features missing) return negative
    // infinity so they fall out of any sort.
    if req.need_tools && !cap.supports_tools {
        return f32::NEG_INFINITY;
    }
    if req.need_vision && !cap.supports_vision {
        return f32::NEG_INFINITY;
    }
    if req.need_json && !cap.supports_json {
        return f32::NEG_INFINITY;
    }
    if cap.context_window < req.min_context {
        return f32::NEG_INFINITY;
    }
    if let Some(max_in) = req.max_cost_in_per_1k_micro_usd
        && cap.cost_in_per_1k_micro_usd > max_in
    {
        return f32::NEG_INFINITY;
    }

    // Composite score:
    //   - quality (0..100) anchors the rank, baseline weight 1.0
    //   - locality match adds +20
    //   - cheaper cost adds up to +15 (full credit at <= 100 µUSD)
    //   - context headroom adds up to +10
    let mut s = cap.quality as f32;
    if let Some(loc) = req.preferred_locality.as_ref()
        && loc == &cap.locality
    {
        s += 20.0;
    }
    let avg_cost = (cap.cost_in_per_1k_micro_usd + cap.cost_out_per_1k_micro_usd) as f32 / 2.0;
    let cost_bonus = if avg_cost <= 100.0 {
        15.0
    } else if avg_cost >= 5000.0 {
        0.0
    } else {
        // Linear taper between $0.0001 and $0.005 per 1k tokens.
        15.0 * (1.0 - (avg_cost - 100.0) / 4900.0)
    };
    s += cost_bonus.clamp(0.0, 15.0);
    let headroom = (cap.context_window as f32 - req.min_context as f32).max(0.0);
    let headroom_bonus = (headroom / 100_000.0).clamp(0.0, 1.0) * 10.0;
    s += headroom_bonus;
    s
}

#[derive(Debug, Default)]
pub struct CapabilityRouter {
    by_provider: HashMap<String, Vec<ModelCapability>>,
}

impl CapabilityRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cap: ModelCapability) {
        self.by_provider
            .entry(cap.provider.clone())
            .or_default()
            .push(cap);
    }

    pub fn len(&self) -> usize {
        self.by_provider.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capabilities(&self) -> Vec<&ModelCapability> {
        self.by_provider.values().flatten().collect()
    }

    /// Return a ranked list of (provider, model, score), highest first.
    /// Models that fail a hard requirement are dropped.
    pub fn rank(&self, req: &CapabilityRequest) -> Vec<Scored<'_>> {
        let mut all: Vec<Scored<'_>> = self
            .capabilities()
            .into_iter()
            .map(|c| Scored {
                cap: c,
                score: score(req, c),
            })
            .filter(|s| s.score.is_finite())
            .collect();
        all.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all
    }

    /// Convenience: top pick or `None`.
    pub fn best(&self, req: &CapabilityRequest) -> Option<&ModelCapability> {
        self.rank(req).first().map(|s| s.cap)
    }
}

/// Default seed catalogue: covers the six MVP providers and is hand-
/// calibrated against late-2026 list-pricing. The router consumer can
/// override at any time via [`CapabilityRouter::register`].
pub fn seed_catalogue() -> Vec<ModelCapability> {
    vec![
        ModelCapability {
            provider: "anthropic".into(),
            model: "claude-opus-4-7".into(),
            locality: Locality::Saas,
            context_window: 200_000,
            supports_tools: true,
            supports_vision: true,
            supports_json: true,
            cost_in_per_1k_micro_usd: 15_000,
            cost_out_per_1k_micro_usd: 75_000,
            quality: 95,
        },
        ModelCapability {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-6".into(),
            locality: Locality::Saas,
            context_window: 200_000,
            supports_tools: true,
            supports_vision: true,
            supports_json: true,
            cost_in_per_1k_micro_usd: 3_000,
            cost_out_per_1k_micro_usd: 15_000,
            quality: 90,
        },
        ModelCapability {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            locality: Locality::Saas,
            context_window: 128_000,
            supports_tools: true,
            supports_vision: true,
            supports_json: true,
            cost_in_per_1k_micro_usd: 5_000,
            cost_out_per_1k_micro_usd: 15_000,
            quality: 88,
        },
        ModelCapability {
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
            locality: Locality::Saas,
            context_window: 128_000,
            supports_tools: true,
            supports_vision: true,
            supports_json: true,
            cost_in_per_1k_micro_usd: 150,
            cost_out_per_1k_micro_usd: 600,
            quality: 78,
        },
        ModelCapability {
            provider: "mistral".into(),
            model: "mistral-large-latest".into(),
            locality: Locality::Saas,
            context_window: 128_000,
            supports_tools: true,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 2_000,
            cost_out_per_1k_micro_usd: 6_000,
            quality: 82,
        },
        ModelCapability {
            provider: "mistral".into(),
            model: "codestral-latest".into(),
            locality: Locality::Saas,
            context_window: 32_000,
            supports_tools: false,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 200,
            cost_out_per_1k_micro_usd: 600,
            quality: 75,
        },
        ModelCapability {
            provider: "ollama".into(),
            model: "llama3.1".into(),
            locality: Locality::Local,
            context_window: 128_000,
            supports_tools: true,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 0,
            cost_out_per_1k_micro_usd: 0,
            quality: 72,
        },
        ModelCapability {
            provider: "ollama".into(),
            model: "qwen2.5".into(),
            locality: Locality::Local,
            context_window: 32_000,
            supports_tools: true,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 0,
            cost_out_per_1k_micro_usd: 0,
            quality: 70,
        },
        ModelCapability {
            provider: "llamacpp".into(),
            model: "gguf".into(),
            locality: Locality::Local,
            context_window: 8_192,
            supports_tools: false,
            supports_vision: false,
            supports_json: false,
            cost_in_per_1k_micro_usd: 0,
            cost_out_per_1k_micro_usd: 0,
            quality: 65,
        },
        ModelCapability {
            provider: "mlx".into(),
            model: "mlx-community/Qwen2.5-7B-Instruct-4bit".into(),
            locality: Locality::Local,
            context_window: 32_000,
            supports_tools: false,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 0,
            cost_out_per_1k_micro_usd: 0,
            quality: 68,
        },
        ModelCapability {
            provider: "groq".into(),
            model: "llama-3.3-70b-versatile".into(),
            locality: Locality::Saas,
            context_window: 128_000,
            supports_tools: true,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 590,
            cost_out_per_1k_micro_usd: 790,
            quality: 82,
        },
        ModelCapability {
            provider: "deepseek".into(),
            model: "deepseek-v4-flash".into(),
            locality: Locality::Saas,
            context_window: 128_000,
            supports_tools: true,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 270,
            cost_out_per_1k_micro_usd: 1_100,
            quality: 84,
        },
    ]
}

/// Build a router populated with [`seed_catalogue`].
pub fn seeded_router() -> CapabilityRouter {
    let mut r = CapabilityRouter::new();
    for c in seed_catalogue() {
        r.register(c);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_catalogue_covers_all_six_providers() {
        let providers: std::collections::HashSet<_> = seed_catalogue()
            .into_iter()
            .map(|c| c.provider)
            .collect();
        for want in ["anthropic", "openai", "mistral", "ollama", "llamacpp", "mlx"] {
            assert!(providers.contains(want), "missing provider in seed: {}", want);
        }
    }

    #[test]
    fn seed_catalogue_covers_groq_and_deepseek() {
        let providers: std::collections::HashSet<_> = seed_catalogue()
            .into_iter()
            .map(|c| c.provider)
            .collect();
        assert!(providers.contains("groq"), "missing groq in seed");
        assert!(providers.contains("deepseek"), "missing deepseek in seed");
    }

    #[test]
    fn score_excludes_models_lacking_required_capability() {
        let req = CapabilityRequest {
            need_vision: true,
            ..Default::default()
        };
        let cap_no_vision = ModelCapability {
            provider: "x".into(),
            model: "no-vision".into(),
            locality: Locality::Local,
            context_window: 32_000,
            supports_tools: true,
            supports_vision: false,
            supports_json: true,
            cost_in_per_1k_micro_usd: 0,
            cost_out_per_1k_micro_usd: 0,
            quality: 90,
        };
        assert!(score(&req, &cap_no_vision).is_infinite());
    }

    #[test]
    fn score_excludes_models_with_too_small_context() {
        let req = CapabilityRequest {
            min_context: 100_000,
            ..Default::default()
        };
        let small = ModelCapability {
            provider: "x".into(),
            model: "small".into(),
            locality: Locality::Local,
            context_window: 8192,
            supports_tools: true,
            supports_vision: true,
            supports_json: true,
            cost_in_per_1k_micro_usd: 0,
            cost_out_per_1k_micro_usd: 0,
            quality: 95,
        };
        assert!(!score(&req, &small).is_finite());
    }

    #[test]
    fn local_preference_adds_locality_bonus() {
        let req_local = CapabilityRequest {
            preferred_locality: Some(Locality::Local),
            ..Default::default()
        };
        let req_saas = CapabilityRequest {
            preferred_locality: Some(Locality::Saas),
            ..Default::default()
        };
        let cap = seed_catalogue()
            .into_iter()
            .find(|c| c.model == "llama3.1")
            .unwrap();
        let s_local = score(&req_local, &cap);
        let s_saas = score(&req_saas, &cap);
        assert!(s_local > s_saas);
    }

    #[test]
    fn router_rank_blends_quality_and_cost_with_no_constraints() {
        let r = seeded_router();
        let req = CapabilityRequest::default();
        let ranked = r.rank(&req);
        assert!(!ranked.is_empty());
        // score() is cost-aware: a cheap frontier model (deepseek-v4-flash,
        // quality 84 + full cost bonus + headroom) outranks the pricier
        // claude-opus-4-7 (quality 95, zero cost bonus) when nothing constrains
        // the request. claude-opus still places near the top on raw quality.
        assert_eq!(ranked[0].cap.model, "deepseek-v4-flash");
        assert!(
            ranked
                .iter()
                .take(3)
                .any(|s| s.cap.model == "claude-opus-4-7"),
            "claude-opus-4-7 should still rank in the top 3 on quality"
        );
    }

    #[test]
    fn router_best_with_local_preference_picks_local_model() {
        let r = seeded_router();
        let req = CapabilityRequest {
            preferred_locality: Some(Locality::Local),
            ..Default::default()
        };
        let best = r.best(&req).unwrap();
        assert_eq!(best.locality, Locality::Local);
    }

    #[test]
    fn router_with_vision_required_drops_codestral() {
        let r = seeded_router();
        let req = CapabilityRequest {
            need_vision: true,
            ..Default::default()
        };
        let ranked = r.rank(&req);
        assert!(ranked.iter().all(|s| s.cap.supports_vision));
        assert!(!ranked.iter().any(|s| s.cap.model == "codestral-latest"));
    }

    #[test]
    fn router_max_cost_filter_kicks_opus_out() {
        let r = seeded_router();
        let req = CapabilityRequest {
            max_cost_in_per_1k_micro_usd: Some(5_000),
            ..Default::default()
        };
        let ranked = r.rank(&req);
        assert!(!ranked.iter().any(|s| s.cap.model == "claude-opus-4-7"));
    }

    #[test]
    fn empty_router_returns_no_picks() {
        let r = CapabilityRouter::new();
        assert!(r.is_empty());
        assert!(r.best(&CapabilityRequest::default()).is_none());
    }

    #[test]
    fn seed_catalogue_has_at_least_ten_rows() {
        assert!(seed_catalogue().len() >= 10);
    }
}

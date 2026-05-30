// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-tuning suggestions — ADR-SELF-IMPROVE-001 §3–4.
//!
//! Turns [`super::observe::Anomaly`]s into operational [`TuningSuggestion`]s
//! over a **constrained change surface** (scheduler weights, cache TTLs,
//! rate limits, resource limits, feature flags, SLO budgets) and never over
//! the forbidden surface (identity/RBAC, network policy, crypto material,
//! user-facing API signatures).
//!
//! Application is opt-in: [`TuningEngine::apply`] only mutates in
//! [`ApplyMode::Apply`] and *always* refuses a forbidden surface regardless
//! of mode. There is no autonomous live change — `Propose` is the default and
//! `Apply` is the explicit gate (cluster flag `CAVE_AGENT_AUTOAPPLY`).

use serde::{Deserialize, Serialize};

use super::observe::{Anomaly, Severity};

/// The surface a tuning change touches. The mutable variants are the only
/// ones the runtime agent is permitted to change (ADR-SELF-IMPROVE-001 §3);
/// the rest are explicitly off-limits (§4) and can never be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeSurface {
    // Mutable surface.
    SchedulerWeight,
    CacheTtl,
    RateLimit,
    ResourceLimit,
    FeatureFlag,
    SloBudget,
    // Forbidden surface.
    Identity,
    NetworkPolicy,
    Crypto,
    ApiSignature,
}

impl ChangeSurface {
    /// Whether the runtime agent may apply a change to this surface.
    pub fn is_mutable(&self) -> bool {
        matches!(
            self,
            ChangeSurface::SchedulerWeight
                | ChangeSurface::CacheTtl
                | ChangeSurface::RateLimit
                | ChangeSurface::ResourceLimit
                | ChangeSurface::FeatureFlag
                | ChangeSurface::SloBudget
        )
    }
}

/// A proposed operational change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuningSuggestion {
    pub target: String,
    pub surface: ChangeSurface,
    pub current: String,
    pub proposed: String,
    pub rationale: String,
    pub severity: Severity,
}

impl TuningSuggestion {
    pub fn new(
        target: impl Into<String>,
        surface: ChangeSurface,
        current: impl Into<String>,
        proposed: impl Into<String>,
        rationale: impl Into<String>,
        severity: Severity,
    ) -> Self {
        Self {
            target: target.into(),
            surface,
            current: current.into(),
            proposed: proposed.into(),
            rationale: rationale.into(),
            severity,
        }
    }
}

/// Whether to merely propose a change or actually apply it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Default — record nothing, hand the suggestion to a human.
    Propose,
    /// Opt-in — apply mutable-surface changes (gated by `CAVE_AGENT_AUTOAPPLY`).
    Apply,
}

/// Result of an [`TuningEngine::apply`] call.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyOutcome {
    Applied,
    Proposed,
    Rejected { reason: String },
}

/// Maps anomalies to suggestions and gates their application.
#[derive(Debug, Default)]
pub struct TuningEngine {
    history: Vec<TuningSuggestion>,
}

impl TuningEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Translate anomalies into suggestions over the constrained surface.
    /// Signals are matched heuristically: latency/p99 → scheduler weight,
    /// error-rate → rate limit, cache → cache TTL.
    pub fn suggest(&self, anomalies: &[Anomaly]) -> Vec<TuningSuggestion> {
        anomalies
            .iter()
            .filter_map(Self::suggestion_for)
            .collect()
    }

    fn suggestion_for(a: &Anomaly) -> Option<TuningSuggestion> {
        let sig = a.signal.to_ascii_lowercase();
        let (surface, target, proposed) = if sig.contains("error_rate") {
            (
                ChangeSurface::RateLimit,
                "ingress.rate_limit_rps",
                "reduce by 20% + extend backoff",
            )
        } else if sig.contains("cache") {
            (
                ChangeSurface::CacheTtl,
                "cache.default_ttl_s",
                "increase TTL to lift hit-ratio",
            )
        } else if sig.contains("p99") || sig.contains("latency") {
            (
                ChangeSurface::SchedulerWeight,
                "scheduler.binpack_weight",
                "shift weight toward spread to relieve hot nodes",
            )
        } else {
            return None;
        };
        Some(TuningSuggestion::new(
            target,
            surface,
            format!("observed={:.2}", a.observed),
            proposed,
            format!(
                "anomaly on '{}' ({:?}): {}",
                a.signal, a.severity, a.detail
            ),
            a.severity,
        ))
    }

    /// Apply or propose a suggestion. A forbidden surface is always
    /// rejected; a mutable surface is applied only in [`ApplyMode::Apply`].
    pub fn apply(&mut self, suggestion: &TuningSuggestion, mode: ApplyMode) -> ApplyOutcome {
        if !suggestion.surface.is_mutable() {
            return ApplyOutcome::Rejected {
                reason: format!(
                    "surface {:?} is off-limits to the runtime agent (ADR-SELF-IMPROVE-001 §4)",
                    suggestion.surface
                ),
            };
        }
        match mode {
            ApplyMode::Propose => ApplyOutcome::Proposed,
            ApplyMode::Apply => {
                self.history.push(suggestion.clone());
                ApplyOutcome::Applied
            }
        }
    }

    /// Every suggestion that was actually applied, in order.
    pub fn history(&self) -> &[TuningSuggestion] {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::self_improve::observe::{Anomaly, Severity};

    fn anomaly(signal: &str, severity: Severity) -> Anomaly {
        Anomaly {
            signal: signal.to_string(),
            observed: 200.0,
            limit: 100.0,
            severity,
            detail: "observed > limit".into(),
        }
    }

    #[test]
    fn latency_anomaly_suggests_scheduler_weight() {
        let s = TuningEngine::new().suggest(&[anomaly("p99:http", Severity::Critical)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].surface, ChangeSurface::SchedulerWeight);
        assert!(s[0].rationale.contains("p99:http"));
    }

    #[test]
    fn error_rate_anomaly_suggests_rate_limit() {
        let s = TuningEngine::new().suggest(&[anomaly("error_rate", Severity::Critical)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].surface, ChangeSurface::RateLimit);
    }

    #[test]
    fn cache_anomaly_suggests_cache_ttl() {
        let s = TuningEngine::new().suggest(&[anomaly("cache_miss_ratio", Severity::Warning)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].surface, ChangeSurface::CacheTtl);
    }

    #[test]
    fn no_anomalies_means_no_suggestions() {
        assert!(TuningEngine::new().suggest(&[]).is_empty());
    }

    #[test]
    fn change_surface_mutability() {
        assert!(ChangeSurface::SchedulerWeight.is_mutable());
        assert!(ChangeSurface::CacheTtl.is_mutable());
        assert!(!ChangeSurface::Identity.is_mutable());
        assert!(!ChangeSurface::Crypto.is_mutable());
        assert!(!ChangeSurface::NetworkPolicy.is_mutable());
        assert!(!ChangeSurface::ApiSignature.is_mutable());
    }

    #[test]
    fn propose_mode_does_not_apply() {
        let mut eng = TuningEngine::new();
        let sug = eng.suggest(&[anomaly("p99:http", Severity::Critical)])[0].clone();
        let outcome = eng.apply(&sug, ApplyMode::Propose);
        assert_eq!(outcome, ApplyOutcome::Proposed);
        assert!(eng.history().is_empty());
    }

    #[test]
    fn apply_mode_applies_mutable_surface_and_records() {
        let mut eng = TuningEngine::new();
        let sug = eng.suggest(&[anomaly("p99:http", Severity::Critical)])[0].clone();
        let outcome = eng.apply(&sug, ApplyMode::Apply);
        assert_eq!(outcome, ApplyOutcome::Applied);
        assert_eq!(eng.history().len(), 1);
    }

    #[test]
    fn forbidden_surface_rejected_even_in_apply_mode() {
        let mut eng = TuningEngine::new();
        let sug = TuningSuggestion::new(
            "system:rbac",
            ChangeSurface::Identity,
            "role=viewer",
            "role=admin",
            "tried to escalate",
            Severity::Critical,
        );
        let outcome = eng.apply(&sug, ApplyMode::Apply);
        assert!(matches!(outcome, ApplyOutcome::Rejected { .. }));
        assert!(eng.history().is_empty(), "forbidden change never recorded");
    }
}

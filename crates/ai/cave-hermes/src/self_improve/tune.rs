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

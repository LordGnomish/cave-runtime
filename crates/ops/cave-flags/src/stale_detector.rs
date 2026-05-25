// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Stale-feature detector — parity with
//! `src/lib/services/scheduler-job/feature-stale-detector` (Unleash v5.0.0).
//!
//! Periodic sweep that marks `stale = true` on feature flags whose
//! `last_seen_at` is older than a per-feature-type threshold.

use crate::models::{FeatureFlag, FeatureType};
use chrono::{DateTime, Duration, Utc};

/// Threshold-by-type table used by the detector.
#[derive(Debug, Clone)]
pub struct StaleThresholds {
    pub release_days: i64,
    pub experiment_days: i64,
    pub operational_days: i64,
    pub kill_switch_days: i64,
    pub permission_days: i64,
}

impl Default for StaleThresholds {
    fn default() -> Self {
        // Unleash defaults: 40d release, 40d experiment, 7d operational,
        // permanent kill-switch / permission (None == never stale).
        Self {
            release_days: 40,
            experiment_days: 40,
            operational_days: 7,
            kill_switch_days: 0,
            permission_days: 0,
        }
    }
}

impl StaleThresholds {
    pub fn threshold_for(&self, t: &FeatureType) -> Option<Duration> {
        let days = match t {
            FeatureType::Release => self.release_days,
            FeatureType::Experiment => self.experiment_days,
            FeatureType::Operational => self.operational_days,
            FeatureType::KillSwitch => self.kill_switch_days,
            FeatureType::Permission => self.permission_days,
        };
        if days <= 0 {
            None
        } else {
            Some(Duration::days(days))
        }
    }
}

/// Detector core — pure function over the cache snapshot.
pub fn detect_stale(
    features: &[FeatureFlag],
    thresholds: &StaleThresholds,
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut newly_stale = Vec::new();
    for f in features {
        if f.stale {
            continue;
        }
        let Some(threshold) = thresholds.threshold_for(&f.feature_type) else {
            continue;
        };
        let last = f.last_seen_at.unwrap_or(f.created_at);
        if now - last > threshold {
            newly_stale.push(f.name.clone());
        }
    }
    newly_stale
}

/// One tick result — returned for telemetry / tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaleSweepResult {
    pub scanned: usize,
    pub marked_stale: Vec<String>,
}

/// Background-job entrypoint. Pure for now (no DB I/O); the runtime
/// `cave_runtime::scheduler` wraps this with a tokio interval ticker
/// and persists marks via `store::mark_stale`.
pub fn run_sweep(
    features: &[FeatureFlag],
    thresholds: &StaleThresholds,
    now: DateTime<Utc>,
) -> StaleSweepResult {
    StaleSweepResult {
        scanned: features.len(),
        marked_stale: detect_stale(features, thresholds, now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FeatureFlag;
    use chrono::TimeZone;

    fn mk_flag(name: &str, t: FeatureType, last_seen: Option<DateTime<Utc>>) -> FeatureFlag {
        FeatureFlag {
            name: name.into(),
            feature_type: t,
            description: String::new(),
            enabled: true,
            stale: false,
            impression_data: false,
            project: "default".into(),
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            last_seen_at: last_seen,
            strategies: vec![],
            variants: vec![],
            environments: vec![],
            tags: vec![],
        }
    }

    #[test]
    fn release_over_40_days_marked_stale() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let flag = mk_flag(
            "old_release",
            FeatureType::Release,
            Some(Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap()),
        );
        let out = detect_stale(&[flag], &StaleThresholds::default(), now);
        assert_eq!(out, vec!["old_release".to_string()]);
    }

    #[test]
    fn release_within_threshold_not_stale() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let flag = mk_flag(
            "fresh_release",
            FeatureType::Release,
            Some(Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap()),
        );
        let out = detect_stale(&[flag], &StaleThresholds::default(), now);
        assert!(out.is_empty());
    }

    #[test]
    fn already_stale_skipped() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let mut flag = mk_flag("already", FeatureType::Release, None);
        flag.stale = true;
        let out = detect_stale(&[flag], &StaleThresholds::default(), now);
        assert!(out.is_empty());
    }

    #[test]
    fn operational_uses_short_threshold() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let flag = mk_flag(
            "ops",
            FeatureType::Operational,
            Some(Utc.with_ymd_and_hms(2026, 5, 20, 0, 0, 0).unwrap()),
        );
        let out = detect_stale(&[flag], &StaleThresholds::default(), now);
        assert_eq!(out, vec!["ops".to_string()]);
    }

    #[test]
    fn kill_switch_never_stale_by_default() {
        let now = Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap();
        let flag = mk_flag(
            "kill",
            FeatureType::KillSwitch,
            Some(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap()),
        );
        let out = detect_stale(&[flag], &StaleThresholds::default(), now);
        assert!(out.is_empty());
    }

    #[test]
    fn missing_last_seen_falls_back_to_created_at() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let flag = mk_flag("no_last_seen", FeatureType::Release, None);
        let out = detect_stale(&[flag], &StaleThresholds::default(), now);
        assert_eq!(out, vec!["no_last_seen".to_string()]);
    }

    #[test]
    fn run_sweep_reports_scanned_count() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let flags = vec![
            mk_flag("a", FeatureType::Release, None),
            mk_flag("b", FeatureType::Release, Some(now)),
        ];
        let r = run_sweep(&flags, &StaleThresholds::default(), now);
        assert_eq!(r.scanned, 2);
        assert_eq!(r.marked_stale, vec!["a".to_string()]);
    }

    #[test]
    fn custom_thresholds_respected() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let flag = mk_flag(
            "experiment_strict",
            FeatureType::Experiment,
            Some(Utc.with_ymd_and_hms(2026, 5, 25, 0, 0, 0).unwrap()),
        );
        let strict = StaleThresholds {
            experiment_days: 3,
            ..StaleThresholds::default()
        };
        let out = detect_stale(&[flag], &strict, now);
        assert_eq!(out, vec!["experiment_strict".to_string()]);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Silence application — Alertmanager silence semantics.

use crate::matcher::all_match;
use crate::models::{Alert, Silence};
use chrono::{DateTime, Utc};

/// Whether the given alert is silenced at `now` by any of the silences.
/// Tenant scoping: only silences from the same tenant apply.
pub fn is_silenced(alert: &Alert, silences: &[Silence], now: DateTime<Utc>) -> bool {
    silences.iter().any(|s| {
        s.tenant_id == alert.tenant_id
            && s.is_active_at(now)
            && all_match(&s.matchers, &alert.labels)
    })
}

/// Annotates each alert's state with `Silenced` if a matching silence is active.
pub fn apply_silences(alerts: &mut [Alert], silences: &[Silence], now: DateTime<Utc>) {
    for a in alerts.iter_mut() {
        if is_silenced(a, silences, now) {
            a.state = crate::models::AlertState::Silenced;
        }
    }
}

/// Number of alerts currently affected by the silence.
pub fn affected_count(silence: &Silence, alerts: &[Alert], now: DateTime<Utc>) -> usize {
    if !silence.is_active_at(now) {
        return 0;
    }
    alerts
        .iter()
        .filter(|a| a.tenant_id == silence.tenant_id && all_match(&silence.matchers, &a.labels))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertSeverity, AlertState, Matcher, Silence};
    use chrono::Duration;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn alert(labels: Vec<(&str, &str)>, fp: &str) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: "X".into(),
            labels: labels
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: fp.into(),
            tenant_id: "anonymous".into(),
            generator_url: None,
        }
    }

    #[test]
    fn test_silence_active_window_matches() {
        let now = Utc::now();
        let s = Silence::new(
            vec![Matcher::equal("env", "prod")],
            now - Duration::minutes(1),
            now + Duration::minutes(10),
            "x",
            "c",
        );
        let a = alert(vec![("env", "prod")], "fp");
        assert!(is_silenced(&a, &[s], now));
    }

    #[test]
    fn test_silence_outside_window_no_match() {
        let now = Utc::now();
        let s = Silence::new(
            vec![Matcher::equal("env", "prod")],
            now - Duration::hours(2),
            now - Duration::hours(1),
            "x",
            "c",
        );
        let a = alert(vec![("env", "prod")], "fp");
        assert!(!is_silenced(&a, &[s], now));
    }

    #[test]
    fn test_silence_label_mismatch_no_match() {
        let now = Utc::now();
        let s = Silence::new(
            vec![Matcher::equal("env", "stage")],
            now - Duration::minutes(1),
            now + Duration::minutes(10),
            "x",
            "c",
        );
        let a = alert(vec![("env", "prod")], "fp");
        assert!(!is_silenced(&a, &[s], now));
    }

    #[test]
    fn test_silence_tenant_isolation() {
        let now = Utc::now();
        let mut s = Silence::new(
            vec![Matcher::equal("env", "prod")],
            now - Duration::minutes(1),
            now + Duration::minutes(10),
            "x",
            "c",
        );
        s.tenant_id = "acme".into();
        let mut a = alert(vec![("env", "prod")], "fp");
        a.tenant_id = "globex".into();
        assert!(!is_silenced(&a, &[s], now));
    }

    #[test]
    fn test_apply_silences_updates_state() {
        let now = Utc::now();
        let s = Silence::new(
            vec![Matcher::equal("env", "prod")],
            now - Duration::minutes(1),
            now + Duration::minutes(10),
            "x",
            "c",
        );
        let mut alerts = vec![
            alert(vec![("env", "prod")], "fp1"),
            alert(vec![("env", "stage")], "fp2"),
        ];
        apply_silences(&mut alerts, &[s], now);
        assert_eq!(alerts[0].state, AlertState::Silenced);
        assert_eq!(alerts[1].state, AlertState::Firing);
    }

    #[test]
    fn test_affected_count() {
        let now = Utc::now();
        let s = Silence::new(
            vec![Matcher::equal("env", "prod")],
            now - Duration::minutes(1),
            now + Duration::minutes(10),
            "x",
            "c",
        );
        let alerts = vec![
            alert(vec![("env", "prod")], "fp1"),
            alert(vec![("env", "prod")], "fp2"),
            alert(vec![("env", "stage")], "fp3"),
        ];
        assert_eq!(affected_count(&s, &alerts, now), 2);
    }
}

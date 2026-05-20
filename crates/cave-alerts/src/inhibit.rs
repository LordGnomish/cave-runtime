// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inhibit rules — Alertmanager `inhibit_rules` semantics.
//!
//! Source alerts whose labels match the rule's `source_matchers` cause target
//! alerts (matching `target_matchers`) to be suppressed when all `equal`
//! labels are equal between source and target.

use crate::matcher::all_match;
use crate::models::{Alert, InhibitRule};

/// Returns `true` if `target` is inhibited by any of the firing `sources`
/// under the given rules. Tenant scoping: only sources from the same tenant
/// can inhibit a target.
pub fn is_inhibited(target: &Alert, sources: &[Alert], rules: &[InhibitRule]) -> bool {
    rules
        .iter()
        .any(|rule| rule_inhibits(rule, target, sources))
}

pub fn rule_inhibits(rule: &InhibitRule, target: &Alert, sources: &[Alert]) -> bool {
    if !all_match(&rule.target_matchers, &target.labels) {
        return false;
    }
    sources.iter().any(|src| {
        src.tenant_id == target.tenant_id
            && src.fingerprint != target.fingerprint
            && all_match(&rule.source_matchers, &src.labels)
            && rule.equal.iter().all(|k| {
                src.labels.get(k).cloned().unwrap_or_default()
                    == target.labels.get(k).cloned().unwrap_or_default()
            })
    })
}

/// Filter a slice of alerts down to those NOT inhibited by any other firing
/// alert under the supplied rules.
pub fn filter_inhibited(alerts: &[Alert], rules: &[InhibitRule]) -> Vec<Alert> {
    alerts
        .iter()
        .filter(|a| !is_inhibited(a, alerts, rules))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertSeverity, AlertState, Matcher};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn alert(name: &str, labels: Vec<(&str, &str)>, fp: &str) -> Alert {
        let mut map: HashMap<String, String> = labels
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        map.entry("alertname".to_string())
            .or_insert_with(|| name.to_string());
        Alert {
            id: Uuid::new_v4(),
            name: name.into(),
            labels: map,
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
    fn test_no_rules_no_inhibit() {
        let target = alert("Target", vec![("severity", "warning")], "fp1");
        assert!(!is_inhibited(&target, &[], &[]));
    }

    #[test]
    fn test_basic_inhibit_with_equal_label() {
        let cluster_down = alert(
            "ClusterDown",
            vec![("cluster", "c1"), ("severity", "critical")],
            "fp-cd",
        );
        let pod_high = alert(
            "PodHigh",
            vec![("cluster", "c1"), ("severity", "warning")],
            "fp-pod",
        );
        let rule = InhibitRule::new(
            "cluster-suppresses-pod",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")],
            vec!["cluster".into()],
        );
        assert!(is_inhibited(
            &pod_high,
            &[cluster_down, pod_high.clone()],
            &[rule]
        ));
    }

    #[test]
    fn test_inhibit_blocked_by_unequal_label() {
        let cluster_down = alert(
            "ClusterDown",
            vec![("cluster", "c1"), ("severity", "critical")],
            "fp-cd",
        );
        let pod_high = alert(
            "PodHigh",
            vec![("cluster", "c2"), ("severity", "warning")],
            "fp-pod",
        );
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")],
            vec!["cluster".into()],
        );
        assert!(!is_inhibited(
            &pod_high,
            &[cluster_down, pod_high.clone()],
            &[rule]
        ));
    }

    #[test]
    fn test_inhibit_target_must_match_target_matchers() {
        let cluster_down = alert(
            "ClusterDown",
            vec![("cluster", "c1"), ("severity", "critical")],
            "fp-cd",
        );
        let pod_high = alert(
            "PodHigh",
            vec![("cluster", "c1"), ("severity", "info")],
            "fp-pod",
        );
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")], // info ≠ warning
            vec!["cluster".into()],
        );
        assert!(!is_inhibited(
            &pod_high,
            &[cluster_down, pod_high.clone()],
            &[rule]
        ));
    }

    #[test]
    fn test_self_does_not_inhibit() {
        let a = alert("X", vec![("cluster", "c1"), ("severity", "warning")], "fp1");
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("severity", "warning")],
            vec![Matcher::equal("severity", "warning")],
            vec!["cluster".into()],
        );
        // Same fingerprint → would self-inhibit. Should not.
        assert!(!is_inhibited(&a, &[a.clone()], &[rule]));
    }

    #[test]
    fn test_filter_inhibited_keeps_sources() {
        let src = alert(
            "ClusterDown",
            vec![("cluster", "c1"), ("severity", "critical")],
            "fp-src",
        );
        let tgt = alert(
            "PodHigh",
            vec![("cluster", "c1"), ("severity", "warning")],
            "fp-tgt",
        );
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")],
            vec!["cluster".into()],
        );
        let kept = filter_inhibited(&[src.clone(), tgt.clone()], &[rule]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].fingerprint, "fp-src");
    }

    #[test]
    fn test_tenant_isolation_in_inhibit() {
        let mut src = alert(
            "ClusterDown",
            vec![("cluster", "c1"), ("severity", "critical")],
            "fp-src",
        );
        src.tenant_id = "acme".into();
        let mut tgt = alert(
            "PodHigh",
            vec![("cluster", "c1"), ("severity", "warning")],
            "fp-tgt",
        );
        tgt.tenant_id = "globex".into();
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")],
            vec!["cluster".into()],
        );
        // Different tenants → no inhibit even though labels match
        assert!(!is_inhibited(&tgt, &[src, tgt.clone()], &[rule]));
    }

    #[test]
    fn test_empty_equal_means_global_inhibit() {
        let src = alert("ClusterDown", vec![("severity", "critical")], "fp-src");
        let tgt = alert("PodHigh", vec![("severity", "warning")], "fp-tgt");
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")],
            vec![], // no equal labels → applies regardless of label overlap
        );
        assert!(is_inhibited(&tgt, &[src, tgt.clone()], &[rule]));
    }
}

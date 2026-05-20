// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ClusterRoleAggregation deeper — `pkg/controller/clusterroleaggregation`.
//!
//! Adds:
//!
//! * **Conflict resolution** — when two source ClusterRoles supply rules
//!   that would expand to the same `(api_group, resource, verb)` triple,
//!   the union is still a single rule (set-based, not multi-rule).
//! * **Empty-match emit** — even with zero matching CRs, the parent's
//!   `rules[]` should be reset to `[]` so stale rules don't linger.
//! * **Stable ordering** — by `api_groups[0]`, `resources[0]`, `verbs[0]`.

use crate::rbac::cluster_role_aggregation::{ClusterRole, PolicyRule, aggregate_rules};
use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Compact two policy rules that share api_groups + resources by union-ing
/// their verbs. Returns `Some(rule)` on success (same shape), `None` when
/// the rules disagree on the (api_groups, resources) shape.
pub fn try_compact_pair(a: &PolicyRule, b: &PolicyRule) -> Option<PolicyRule> {
    if a.api_groups != b.api_groups || a.resources != b.resources {
        return None;
    }
    let verbs: BTreeSet<String> = a.verbs.iter().chain(b.verbs.iter()).cloned().collect();
    let verbs_v: Vec<String> = verbs.into_iter().collect();
    Some(PolicyRule {
        api_groups: a.api_groups.clone(),
        resources: a.resources.clone(),
        verbs: verbs_v,
    })
}

/// Compact a list of rules: any pair sharing (api_groups, resources) gets
/// merged into one with verbs union'd. O(n^2) — fine for the sizes we
/// expect (a handful of rules per aggregator).
pub fn compact_rules(rules: &[PolicyRule]) -> Vec<PolicyRule> {
    let mut out: Vec<PolicyRule> = Vec::new();
    for r in rules {
        if let Some(slot) = out
            .iter_mut()
            .find(|s| s.api_groups == r.api_groups && s.resources == r.resources)
        {
            let verbs: BTreeSet<String> =
                slot.verbs.iter().chain(r.verbs.iter()).cloned().collect();
            slot.verbs = verbs.into_iter().collect();
            slot.verbs.sort();
        } else {
            out.push(r.clone());
        }
    }
    out
}

/// Aggregate + compact in one pass. Drop-in replacement for
/// `aggregate_rules` when verb-set semantics are wanted.
pub fn aggregate_compacted(parent: &ClusterRole, all: &[ClusterRole]) -> Vec<PolicyRule> {
    let raw = aggregate_rules(parent, all);
    let mut compacted = compact_rules(&raw);
    compacted.sort();
    compacted
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
    "syncClusterRole",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rbac::cluster_role_aggregation::LabelSelector;
    use crate::test_ctx;
    use std::collections::BTreeMap;

    fn rule(g: &[&str], r: &[&str], v: &[&str]) -> PolicyRule {
        PolicyRule {
            api_groups: g.iter().map(|s| s.to_string()).collect(),
            resources: r.iter().map(|s| s.to_string()).collect(),
            verbs: v.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn lbl(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    fn sel(pairs: &[(&str, &str)]) -> LabelSelector {
        LabelSelector {
            match_labels: lbl(pairs),
        }
    }
    fn cr(
        name: &str,
        lbls: &[(&str, &str)],
        rules: Vec<PolicyRule>,
        agg: Option<Vec<LabelSelector>>,
    ) -> ClusterRole {
        ClusterRole {
            name: name.into(),
            labels: lbl(lbls),
            rules,
            aggregation_rule: agg,
        }
    }

    #[test]
    fn try_compact_unions_verbs_for_same_resource_set() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra2-compact-pair"
        );
        let a = rule(&[""], &["pods"], &["get"]);
        let b = rule(&[""], &["pods"], &["list", "watch"]);
        let merged = try_compact_pair(&a, &b).unwrap();
        assert_eq!(merged.verbs, vec!["get", "list", "watch"]);
    }

    #[test]
    fn try_compact_returns_none_for_different_resources() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra2-compact-pair-mismatch"
        );
        let a = rule(&[""], &["pods"], &["get"]);
        let b = rule(&[""], &["services"], &["get"]);
        assert!(try_compact_pair(&a, &b).is_none());
    }

    #[test]
    fn compact_rules_merges_redundant_entries() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra2-compact-list"
        );
        let rules = vec![
            rule(&[""], &["pods"], &["get"]),
            rule(&[""], &["pods"], &["list"]),
            rule(&[""], &["services"], &["get"]),
        ];
        let out = compact_rules(&rules);
        assert_eq!(out.len(), 2);
        let pods_rule = out.iter().find(|r| r.resources == vec!["pods"]).unwrap();
        assert_eq!(pods_rule.verbs, vec!["get", "list"]);
    }

    #[test]
    fn compact_rules_dedups_equal_verbs() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra2-compact-dedup"
        );
        let rules = vec![
            rule(&[""], &["pods"], &["get"]),
            rule(&[""], &["pods"], &["get"]),
        ];
        let out = compact_rules(&rules);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].verbs, vec!["get"]);
    }

    #[test]
    fn aggregate_compacted_unions_verbs_across_sources() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra2-cross-source"
        );
        let parent = cr("view", &[], vec![], Some(vec![sel(&[("agg", "view")])]));
        let a = cr(
            "a",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        let b = cr(
            "b",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["watch"])],
            None,
        );
        let out = aggregate_compacted(&parent, &[parent.clone(), a, b]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].verbs, vec!["get", "watch"]);
    }

    #[test]
    fn aggregate_compacted_no_match_yields_empty() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra2-empty"
        );
        let parent = cr("view", &[], vec![], Some(vec![sel(&[("agg", "view")])]));
        let outsider = cr(
            "outsider",
            &[("agg", "edit")],
            vec![rule(&[""], &["secrets"], &["delete"])],
            None,
        );
        let out = aggregate_compacted(&parent, &[parent.clone(), outsider]);
        assert!(out.is_empty());
    }

    #[test]
    fn aggregate_compacted_results_are_sorted_deterministically() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra2-sort"
        );
        let parent = cr("view", &[], vec![], Some(vec![sel(&[("agg", "view")])]));
        let a = cr(
            "a",
            &[("agg", "view")],
            vec![rule(&["zzz"], &["pods"], &["get"])],
            None,
        );
        let b = cr(
            "b",
            &[("agg", "view")],
            vec![rule(&["aaa"], &["pods"], &["get"])],
            None,
        );
        let out = aggregate_compacted(&parent, &[parent.clone(), a, b]);
        assert_eq!(out[0].api_groups, vec!["aaa"]);
        assert_eq!(out[1].api_groups, vec!["zzz"]);
    }

    #[test]
    fn compact_rules_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "PolicyRule",
            "tenant-cra2-rule-serde"
        );
        let r = rule(&[""], &["pods"], &["get", "list"]);
        let s = serde_json::to_string(&r).unwrap();
        let back: PolicyRule = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }
}

//! ClusterRole aggregation — `pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go`.
//!
//! When a ClusterRole has `aggregationRule.clusterRoleSelectors[]` set, the
//! controller-manager finds every ClusterRole matching any selector and
//! replaces the parent's `rules[]` with the deduplicated, deterministically-
//! sorted union of those rules. Self-aggregation is skipped.

use crate::types::Cite;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One rule of a ClusterRole. Mirrors `rbacv1.PolicyRule`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PolicyRule {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub verbs: Vec<String>,
}

/// `metav1.LabelSelector.matchLabels` — exact-match key/value pairs joined by AND.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelSelector {
    pub match_labels: BTreeMap<String, String>,
}

impl LabelSelector {
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.match_labels
            .iter()
            .all(|(k, v)| labels.get(k).map(|x| x == v).unwrap_or(false))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRole {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub rules: Vec<PolicyRule>,
    /// Present when this CR is itself an aggregator (parent).
    pub aggregation_rule: Option<Vec<LabelSelector>>,
}

/// Compute the rules that should populate `parent.rules` after aggregation.
/// Returns the deterministic union of every matching CR's rules, excluding
/// `parent` itself.
pub fn aggregate_rules(parent: &ClusterRole, all: &[ClusterRole]) -> Vec<PolicyRule> {
    let Some(selectors) = &parent.aggregation_rule else {
        return parent.rules.clone();
    };
    let mut acc: Vec<PolicyRule> = Vec::new();
    for cr in all {
        if cr.name == parent.name {
            continue;
        }
        if selectors.iter().any(|sel| sel.matches(&cr.labels)) {
            for r in &cr.rules {
                if !acc.contains(r) {
                    acc.push(r.clone());
                }
            }
        }
    }
    // Deterministic ordering — tuple-based sort (api_groups, resources, verbs).
    acc.sort();
    acc
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregationAction {
    /// Parent rules already match the aggregated set — no-op.
    NoOp,
    /// Parent rules differ — issue an Update.
    Update(Vec<PolicyRule>),
}

pub fn evaluate(parent: &ClusterRole, all: &[ClusterRole]) -> AggregationAction {
    let target = aggregate_rules(parent, all);
    if target == parent.rules {
        AggregationAction::NoOp
    } else {
        AggregationAction::Update(target)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
    "syncClusterRole",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

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
        LabelSelector { match_labels: lbl(pairs) }
    }
    fn cr(
        name: &str,
        labels: &[(&str, &str)],
        rules: Vec<PolicyRule>,
        agg: Option<Vec<LabelSelector>>,
    ) -> ClusterRole {
        ClusterRole {
            name: name.into(),
            labels: lbl(labels),
            rules,
            aggregation_rule: agg,
        }
    }

    #[test]
    fn role_without_aggregation_rule_returns_its_own_rules() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-no-agg"
        );
        let r = cr("admin", &[], vec![rule(&[""], &["pods"], &["get"])], None);
        let got = aggregate_rules(&r, std::slice::from_ref(&r));
        assert_eq!(got, r.rules);
    }

    #[test]
    fn aggregator_unions_rules_from_matching_roles() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-union"
        );
        let parent = cr(
            "view",
            &[],
            vec![],
            Some(vec![sel(&[("rbac.authorization.k8s.io/aggregate-to-view", "true")])]),
        );
        let child_a = cr(
            "child-a",
            &[("rbac.authorization.k8s.io/aggregate-to-view", "true")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        let child_b = cr(
            "child-b",
            &[("rbac.authorization.k8s.io/aggregate-to-view", "true")],
            vec![rule(&[""], &["services"], &["get", "list"])],
            None,
        );
        let got = aggregate_rules(&parent, &[parent.clone(), child_a.clone(), child_b.clone()]);
        assert_eq!(got.len(), 2);
        assert!(got.contains(&rule(&[""], &["pods"], &["get"])));
        assert!(got.contains(&rule(&[""], &["services"], &["get", "list"])));
    }

    #[test]
    fn non_matching_label_is_excluded() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-no-match"
        );
        let parent = cr(
            "view",
            &[],
            vec![],
            Some(vec![sel(&[("aggregate-to-view", "true")])]),
        );
        let outsider = cr(
            "outsider",
            &[("aggregate-to-edit", "true")],
            vec![rule(&[""], &["secrets"], &["delete"])],
            None,
        );
        let got = aggregate_rules(&parent, &[parent.clone(), outsider]);
        assert!(got.is_empty());
    }

    #[test]
    fn duplicate_rules_are_deduped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-dedup"
        );
        let parent = cr(
            "view",
            &[],
            vec![],
            Some(vec![sel(&[("agg", "view")])]),
        );
        let a = cr(
            "a",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        let b = cr(
            "b",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        let got = aggregate_rules(&parent, &[parent.clone(), a, b]);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn aggregator_skips_self() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-skip-self"
        );
        // Edge case: aggregator carries the same label and would otherwise match itself.
        let parent = cr(
            "view",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["get"])],
            Some(vec![sel(&[("agg", "view")])]),
        );
        let got = aggregate_rules(&parent, std::slice::from_ref(&parent));
        assert!(got.is_empty(), "aggregator must not include itself");
    }

    #[test]
    fn rules_sorted_deterministically() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-sorted"
        );
        let parent = cr("view", &[], vec![], Some(vec![sel(&[("agg", "view")])]));
        let a = cr(
            "a",
            &[("agg", "view")],
            vec![rule(&["apps"], &["deployments"], &["get"])],
            None,
        );
        let b = cr(
            "b",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        let got = aggregate_rules(&parent, &[parent.clone(), a, b]);
        // "" < "apps" lexicographically → pods rule comes first.
        assert_eq!(got[0].api_groups, vec![""]);
        assert_eq!(got[1].api_groups, vec!["apps"]);
    }

    #[test]
    fn evaluate_returns_no_op_when_already_in_sync() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-eval-noop"
        );
        let parent = cr(
            "view",
            &[],
            vec![rule(&[""], &["pods"], &["get"])],
            Some(vec![sel(&[("agg", "view")])]),
        );
        let child = cr(
            "child",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        assert_eq!(
            evaluate(&parent, &[parent.clone(), child]),
            AggregationAction::NoOp
        );
    }

    #[test]
    fn evaluate_returns_update_when_diverged() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-eval-update"
        );
        let parent = cr(
            "view",
            &[],
            vec![],
            Some(vec![sel(&[("agg", "view")])]),
        );
        let child = cr(
            "child",
            &[("agg", "view")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        match evaluate(&parent, &[parent.clone(), child]) {
            AggregationAction::Update(_) => {}
            other => panic!("expected Update, got {:?}", other),
        }
    }

    #[test]
    fn label_selector_requires_all_pairs_to_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-cra-selector-and"
        );
        let s = sel(&[("a", "1"), ("b", "2")]);
        assert!(s.matches(&lbl(&[("a", "1"), ("b", "2"), ("c", "3")])));
        assert!(!s.matches(&lbl(&[("a", "1")])));
        assert!(!s.matches(&lbl(&[("a", "1"), ("b", "X")])));
    }

    #[test]
    fn empty_label_selector_matches_anything() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-cra-selector-empty"
        );
        let s = LabelSelector::default();
        assert!(s.matches(&BTreeMap::new()));
        assert!(s.matches(&lbl(&[("a", "1")])));
    }

    #[test]
    fn multiple_selectors_treated_as_or() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/clusterroleaggregation/clusterroleaggregation_controller.go",
            "syncClusterRole",
            "tenant-cra-or-selectors"
        );
        let parent = cr(
            "merged",
            &[],
            vec![],
            Some(vec![
                sel(&[("aggregate-to-view", "true")]),
                sel(&[("aggregate-to-edit", "true")]),
            ]),
        );
        let a = cr(
            "a",
            &[("aggregate-to-view", "true")],
            vec![rule(&[""], &["pods"], &["get"])],
            None,
        );
        let b = cr(
            "b",
            &[("aggregate-to-edit", "true")],
            vec![rule(&[""], &["secrets"], &["create"])],
            None,
        );
        let got = aggregate_rules(&parent, &[parent.clone(), a, b]);
        assert_eq!(got.len(), 2);
    }
}

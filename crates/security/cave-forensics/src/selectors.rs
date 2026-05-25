// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pod / container / namespace selectors for tracing policies.
//!
//! Upstream: `pkg/podhooks/podhooks.go`, `pkg/selectors/kernel_selectors.go`,
//! `pkg/k8s/apis/cilium.io/v1alpha1/selector_types.go`.

use crate::error::{ForensicsError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Matches a Kubernetes Pod by labels + optional namespace list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PodSelector {
    #[serde(default)]
    pub match_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub match_expressions: Vec<LabelExpression>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LabelExpression {
    pub key: String,
    pub operator: SelectorOp,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum SelectorOp {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

/// Matches a single container inside a pod by name + image substring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ContainerSelector {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub image_globs: Vec<String>,
}

/// Snapshot of a runtime pod that a selector is evaluated against.
#[derive(Debug, Clone, PartialEq)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub labels: BTreeMap<String, String>,
    pub containers: Vec<ContainerInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
}

impl PodSelector {
    /// Evaluate this selector against a [`PodInfo`]. Empty selector matches
    /// everything (mirrors `metav1.LabelSelector{}`).
    pub fn matches(&self, pod: &PodInfo) -> bool {
        if !self.namespaces.is_empty() && !self.namespaces.iter().any(|n| n == &pod.namespace) {
            return false;
        }
        for (k, v) in &self.match_labels {
            match pod.labels.get(k) {
                Some(pv) if pv == v => {}
                _ => return false,
            }
        }
        for expr in &self.match_expressions {
            if !expr.evaluate(&pod.labels) {
                return false;
            }
        }
        true
    }
}

impl LabelExpression {
    /// Evaluate one expression against a label map.
    pub fn evaluate(&self, labels: &BTreeMap<String, String>) -> bool {
        match self.operator {
            SelectorOp::In => labels
                .get(&self.key)
                .is_some_and(|v| self.values.iter().any(|x| x == v)),
            SelectorOp::NotIn => labels
                .get(&self.key)
                .is_none_or(|v| !self.values.iter().any(|x| x == v)),
            SelectorOp::Exists => labels.contains_key(&self.key),
            SelectorOp::DoesNotExist => !labels.contains_key(&self.key),
        }
    }

    /// Reject invalid In/NotIn (require non-empty values).
    pub fn validate(&self) -> Result<()> {
        if matches!(self.operator, SelectorOp::In | SelectorOp::NotIn) && self.values.is_empty() {
            return Err(ForensicsError::InvalidSelector(format!(
                "{:?} requires non-empty values",
                self.operator
            )));
        }
        Ok(())
    }
}

impl ContainerSelector {
    /// True if at least one container in `pod` matches this selector.
    /// Empty selector matches every container.
    pub fn matches_any(&self, pod: &PodInfo) -> bool {
        pod.containers.iter().any(|c| self.matches(c))
    }

    /// Match a single container.
    pub fn matches(&self, c: &ContainerInfo) -> bool {
        if !self.names.is_empty() && !self.names.iter().any(|n| n == &c.name) {
            return false;
        }
        if !self.image_globs.is_empty() && !self.image_globs.iter().any(|g| glob_match(g, &c.image))
        {
            return false;
        }
        true
    }
}

/// Tiny `*`-only glob matcher (no `?`, no char classes — sufficient for
/// image refs as Tetragon uses them).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut cursor = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !text[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
        } else if i == parts.len() - 1 {
            if !text[cursor..].ends_with(part) {
                return false;
            }
            return text.len() >= cursor + part.len();
        } else {
            match text[cursor..].find(part) {
                Some(p) => cursor += p + part.len(),
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod() -> PodInfo {
        let mut labels = BTreeMap::new();
        labels.insert("app".into(), "nginx".into());
        labels.insert("tier".into(), "frontend".into());
        PodInfo {
            name: "nginx-1".into(),
            namespace: "default".into(),
            labels,
            containers: vec![
                ContainerInfo {
                    name: "main".into(),
                    image: "nginx:1.25".into(),
                },
                ContainerInfo {
                    name: "sidecar".into(),
                    image: "envoyproxy/envoy:v1.34".into(),
                },
            ],
        }
    }

    #[test]
    fn test_empty_pod_selector_matches_all() {
        assert!(PodSelector::default().matches(&pod()));
    }

    #[test]
    fn test_namespace_filter() {
        let mut s = PodSelector::default();
        s.namespaces = vec!["kube-system".into()];
        assert!(!s.matches(&pod()));
        s.namespaces = vec!["default".into(), "kube-system".into()];
        assert!(s.matches(&pod()));
    }

    #[test]
    fn test_match_labels_exact() {
        let mut s = PodSelector::default();
        s.match_labels.insert("app".into(), "nginx".into());
        assert!(s.matches(&pod()));
        s.match_labels.insert("app".into(), "redis".into());
        assert!(!s.matches(&pod()));
    }

    #[test]
    fn test_match_expressions_in() {
        let mut s = PodSelector::default();
        s.match_expressions.push(LabelExpression {
            key: "tier".into(),
            operator: SelectorOp::In,
            values: vec!["frontend".into(), "backend".into()],
        });
        assert!(s.matches(&pod()));
    }

    #[test]
    fn test_match_expressions_notin() {
        let mut s = PodSelector::default();
        s.match_expressions.push(LabelExpression {
            key: "tier".into(),
            operator: SelectorOp::NotIn,
            values: vec!["frontend".into()],
        });
        assert!(!s.matches(&pod()));
    }

    #[test]
    fn test_match_expressions_exists() {
        let mut s = PodSelector::default();
        s.match_expressions.push(LabelExpression {
            key: "app".into(),
            operator: SelectorOp::Exists,
            values: vec![],
        });
        assert!(s.matches(&pod()));
    }

    #[test]
    fn test_match_expressions_doesnotexist() {
        let mut s = PodSelector::default();
        s.match_expressions.push(LabelExpression {
            key: "missing-key".into(),
            operator: SelectorOp::DoesNotExist,
            values: vec![],
        });
        assert!(s.matches(&pod()));
    }

    #[test]
    fn test_label_expression_validate_in_requires_values() {
        let e = LabelExpression {
            key: "x".into(),
            operator: SelectorOp::In,
            values: vec![],
        };
        assert!(e.validate().is_err());
    }

    #[test]
    fn test_container_selector_by_name() {
        let mut cs = ContainerSelector::default();
        cs.names = vec!["main".into()];
        assert!(cs.matches_any(&pod()));
        cs.names = vec!["nope".into()];
        assert!(!cs.matches_any(&pod()));
    }

    #[test]
    fn test_container_selector_by_image_glob() {
        let mut cs = ContainerSelector::default();
        cs.image_globs = vec!["nginx:*".into()];
        assert!(cs.matches_any(&pod()));
        cs.image_globs = vec!["redis:*".into()];
        assert!(!cs.matches_any(&pod()));
    }

    #[test]
    fn test_glob_simple_star() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("nginx:*", "nginx:1.25"));
        assert!(!glob_match("nginx:*", "redis:7"));
    }

    #[test]
    fn test_glob_exact() {
        assert!(glob_match("nginx:1.25", "nginx:1.25"));
        assert!(!glob_match("nginx:1.25", "nginx:1.26"));
    }

    #[test]
    fn test_glob_prefix_suffix() {
        assert!(glob_match("*envoy*", "envoyproxy/envoy:v1.34"));
        assert!(glob_match("envoyproxy/*", "envoyproxy/envoy:v1.34"));
        assert!(glob_match("*:v1.34", "envoyproxy/envoy:v1.34"));
    }

    #[test]
    fn test_namespaces_serde() {
        let s = PodSelector {
            namespaces: vec!["a".into()],
            ..Default::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: PodSelector = serde_json::from_str(&j).unwrap();
        assert_eq!(back.namespaces, vec!["a".to_string()]);
    }
}

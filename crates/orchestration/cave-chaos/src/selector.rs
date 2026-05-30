// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Chaos target selector — Chaos Mesh `api/v1alpha1/selector.go` +
//! `controllers/utils/selector` port.
//!
//! Resolves the set of pods a chaos experiment will affect: namespace scoping,
//! label-selector matching (`matchLabels` + `matchExpressions`), and `mode`
//! reduction (`one`/`all`/`fixed`/`fixed-percent`/`random-max-percent`).
//!
//! Selection is **deterministic** here (first-k in input order): the live
//! random tie-break in Chaos Mesh's `random-max-percent`/`one` is part of the
//! privileged daemon path and is out of scope for this in-process port — what
//! matters for parity is the exact subset *size* formula, which this preserves.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A candidate pod considered for fault injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub labels: HashMap<String, String>,
    /// Whether the pod is currently healthy (used by the safety/health gate).
    pub healthy: bool,
}

/// How matched pods are reduced to the injection subset.
/// Maps to Chaos Mesh `spec.selector.mode` + `value`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "mode", content = "value")]
pub enum SelectorMode {
    /// Exactly one pod.
    One,
    /// Every matched pod.
    All,
    /// An absolute count (capped at the matched pool size).
    Fixed(usize),
    /// A percentage of the matched pool, rounded **up** (ceiling).
    FixedPercent(u8),
    /// An upper-bound percentage of the matched pool, rounded **down** (floor).
    RandomMaxPercent(u8),
}

/// A `matchExpressions` operator.
/// Maps to Kubernetes `LabelSelectorRequirement` operators.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LabelExpr {
    /// Label value must be one of `values`.
    In(String, Vec<String>),
    /// Label value must not be any of `values` (absent key passes).
    NotIn(String, Vec<String>),
    /// Label key must be present (any value).
    Exists(String),
    /// Label key must be absent.
    DoesNotExist(String),
}

impl LabelExpr {
    fn matches(&self, labels: &HashMap<String, String>) -> bool {
        match self {
            LabelExpr::In(key, values) => labels
                .get(key)
                .map(|v| values.contains(v))
                .unwrap_or(false),
            LabelExpr::NotIn(key, values) => labels
                .get(key)
                .map(|v| !values.contains(v))
                .unwrap_or(true), // absent key passes NotIn
            LabelExpr::Exists(key) => labels.contains_key(key),
            LabelExpr::DoesNotExist(key) => !labels.contains_key(key),
        }
    }
}

/// A label selector — `matchLabels` (equality, AND) plus `matchExpressions`
/// (operators, AND). All constraints must hold for a pod to match.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LabelSelector {
    pub match_labels: HashMap<String, String>,
    pub match_expressions: Vec<LabelExpr>,
}

impl LabelSelector {
    /// Returns `true` iff the pod satisfies every `matchLabels` pair and every
    /// `matchExpressions` requirement.
    pub fn matches(&self, pod: &PodInfo) -> bool {
        for (k, v) in &self.match_labels {
            if pod.labels.get(k) != Some(v) {
                return false;
            }
        }
        for expr in &self.match_expressions {
            if !expr.matches(&pod.labels) {
                return false;
            }
        }
        true
    }
}

/// Exact subset size for a mode over a matched pool of `n` pods.
///
/// * `One`               → `min(1, n)`
/// * `All`               → `n`
/// * `Fixed(v)`          → `min(v, n)`
/// * `FixedPercent(p)`   → `min(ceil(n*p/100), n)`
/// * `RandomMaxPercent(p)` → `min(floor(n*p/100), n)`
pub fn subset_size(mode: &SelectorMode, n: usize) -> usize {
    match mode {
        SelectorMode::One => n.min(1),
        SelectorMode::All => n,
        SelectorMode::Fixed(v) => (*v).min(n),
        SelectorMode::FixedPercent(p) => {
            let p = *p as usize;
            let ceil = (n * p).div_ceil(100);
            ceil.min(n)
        }
        SelectorMode::RandomMaxPercent(p) => {
            let p = *p as usize;
            let floor = (n * p) / 100;
            floor.min(n)
        }
    }
}

/// Resolve the injection target set for an experiment.
///
/// Flow (Chaos Mesh order): (1) namespace filter (skipped when `namespaces` is
/// empty → all namespaces), (2) label selector, (3) `mode` reduction to the
/// first-k matched pods (deterministic, stable input order).
pub fn resolve_targets(
    pods: &[PodInfo],
    namespaces: &[String],
    selector: &LabelSelector,
    mode: &SelectorMode,
) -> Vec<PodInfo> {
    let matched: Vec<&PodInfo> = pods
        .iter()
        .filter(|p| namespaces.is_empty() || namespaces.contains(&p.namespace))
        .filter(|p| selector.matches(p))
        .collect();
    let k = subset_size(mode, matched.len());
    matched.into_iter().take(k).cloned().collect()
}

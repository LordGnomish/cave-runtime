// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/scheduling/requirements.go from kubernetes-sigs/karpenter
// v1.12.1 (sha ed490e8). Apache-2.0 upstream; see NOTICE.
//
//! `Requirements` is a keyed collection of [`Requirement`], one per label
//! key. Adding a requirement for an existing key intersects the two. The
//! collection drives scheduling compatibility checks.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::requirement::{NodeSelectorRequirement, Operator, Requirement};

/// Well-known label keys. When `AllowUndefinedWellKnownLabels` is set these
/// keys are permitted to be undefined on the candidate side; they also seed
/// the typo-detection hint. Mirrors `v1.WellKnownLabels`.
fn well_known_labels() -> &'static [&'static str] {
    &[
        "topology.kubernetes.io/zone",
        "topology.kubernetes.io/region",
        "node.kubernetes.io/instance-type",
        "kubernetes.io/arch",
        "kubernetes.io/os",
        "kubernetes.io/hostname",
        "karpenter.sh/capacity-type",
        "karpenter.sh/nodepool",
    ]
}

/// Labels excluded from the human-readable `String()` form. Mirrors
/// `v1.RestrictedLabels`.
fn is_restricted_label(key: &str) -> bool {
    matches!(
        key,
        "karpenter.sh/nodepool"
            | "karpenter.sh/nodeclaim"
            | "kubernetes.io/hostname"
    )
}

#[derive(Debug, Clone, Default)]
pub struct Requirements {
    inner: BTreeMap<String, Requirement>,
}

impl Requirements {
    /// `NewRequirements` — construct from a list, intersecting on collision.
    pub fn new(requirements: Vec<Requirement>) -> Requirements {
        let mut r = Requirements {
            inner: BTreeMap::new(),
        };
        for req in requirements {
            r.add(req);
        }
        r
    }

    /// `NewLabelRequirements` — one `In` requirement per label.
    pub fn from_labels(labels: &BTreeMap<String, String>) -> Requirements {
        let mut r = Requirements::new(vec![]);
        for (k, v) in labels {
            r.add(Requirement::new(k, Operator::In, &[v.clone()]));
        }
        r
    }

    /// `Add` — intersect with any existing requirement for the same key.
    pub fn add(&mut self, requirement: Requirement) {
        let req = match self.inner.get(&requirement.key) {
            Some(existing) => requirement.intersection(existing),
            None => requirement,
        };
        self.inner.insert(req.key.clone(), req);
    }

    /// `Keys` — the set of label keys present.
    pub fn keys(&self) -> BTreeSet<String> {
        self.inner.keys().cloned().collect()
    }

    pub fn values(&self) -> Vec<&Requirement> {
        self.inner.values().collect()
    }

    pub fn has(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    /// `Get` — the requirement for `key`, or an Exists requirement (allow any)
    /// when the key is undefined.
    pub fn get(&self, key: &str) -> Requirement {
        match self.inner.get(key) {
            Some(r) => r.clone(),
            None => Requirement::new(key, Operator::Exists, &[]),
        }
    }

    pub fn has_min_values(&self) -> bool {
        self.inner.values().any(|r| r.min_values.is_some())
    }

    /// `NodeSelectorRequirements` — serialized form; a requirement with both
    /// bounds expands to two entries (Gte + Lte).
    pub fn node_selector_requirements(&self) -> Vec<NodeSelectorRequirement> {
        let mut out = Vec::with_capacity(self.inner.len());
        for req in self.inner.values() {
            out.extend(req.bounded_node_selector_requirements());
        }
        out
    }

    /// `IsCompatible` — convenience bool form of [`Requirements::compatible`].
    pub fn is_compatible(&self, requirements: &Requirements, allow_undefined_well_known: bool) -> bool {
        self.compatible(requirements, allow_undefined_well_known).is_ok()
    }

    /// `Compatible` — ensure the provided requirements can loosely be met.
    /// Custom labels must intersect (undefined → denied unless the incoming
    /// operator is NotIn / DoesNotExist). Well-known labels may be undefined
    /// when `allow_undefined_well_known` is set. Then the value sets must
    /// intersect.
    pub fn compatible(
        &self,
        requirements: &Requirements,
        allow_undefined_well_known: bool,
    ) -> Result<(), String> {
        for key in requirements.inner.keys() {
            if allow_undefined_well_known && well_known_labels().contains(&key.as_str()) {
                continue;
            }
            let operator = requirements.get(key).operator();
            if self.has(key)
                || operator == Operator::NotIn
                || operator == Operator::DoesNotExist
            {
                continue;
            }
            // Report only the first error.
            return Err(format!(
                "label {:?} does not have known values{}",
                key,
                self.label_hint(key, allow_undefined_well_known)
            ));
        }
        self.intersects(requirements)
    }

    /// `Intersects` — the value sets of shared keys must overlap. Undefined
    /// keys are allowed. Two negative operators (NotIn / DoesNotExist) that
    /// don't share values are still compatible.
    pub fn intersects(&self, requirements: &Requirements) -> Result<(), String> {
        let mut errs: Vec<String> = vec![];
        for key in self.intersect_keys(requirements) {
            let existing = self.get(&key);
            let incoming = requirements.get(&key);
            if !existing.has_intersection(&incoming) {
                let inc_op = incoming.operator();
                if inc_op == Operator::NotIn || inc_op == Operator::DoesNotExist {
                    let ex_op = existing.operator();
                    if ex_op == Operator::NotIn || ex_op == Operator::DoesNotExist {
                        continue;
                    }
                }
                errs.push(format!(
                    "key {}, {} not in {}",
                    key, incoming, existing
                ));
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("; "))
        }
    }

    fn intersect_keys(&self, rhs: &Requirements) -> BTreeSet<String> {
        let (smallest, largest) = if self.inner.len() > rhs.inner.len() {
            (&rhs.inner, &self.inner)
        } else {
            (&self.inner, &rhs.inner)
        };
        smallest
            .keys()
            .filter(|k| largest.contains_key(*k))
            .cloned()
            .collect()
    }

    /// `labelHint` — suggest a well-known (or existing) label the bad key may
    /// be a typo of. Deterministic: iterates well-known labels in sorted
    /// order.
    fn label_hint(&self, key: &str, allow_undefined_well_known: bool) -> String {
        if allow_undefined_well_known {
            let mut wk: Vec<&str> = well_known_labels().to_vec();
            wk.sort_unstable();
            for well_known in wk {
                if well_known.contains(key) {
                    return format!(" (typo of {well_known:?}?)");
                }
                if well_known.ends_with(suffix(key)) {
                    return format!(" (typo of {well_known:?}?)");
                }
            }
        }
        for existing in self.inner.keys() {
            if existing.contains(key) {
                return format!(" (typo of {existing:?}?)");
            }
            if existing.ends_with(suffix(key)) {
                return format!(" (typo of {existing:?}?)");
            }
        }
        String::new()
    }
}

impl std::fmt::Display for Requirements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts: Vec<String> = self
            .inner
            .values()
            .filter(|req| !is_restricted_label(&req.key))
            .map(|req| req.to_string())
            .collect();
        parts.sort();
        f.write_str(&parts.join(", "))
    }
}

/// The part of `key` after the first `/`, or the whole key when there is none.
fn suffix(key: &str) -> &str {
    match key.split_once('/') {
        Some((_, after)) => after,
        None => key,
    }
}

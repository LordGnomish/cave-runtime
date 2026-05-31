// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Usage protection — gate deletion of a resource that is in use by another.
//!
//! Upstream: crossplane/crossplane v2.3.1
//!   - apis/protection/v1beta1/usage_types.go
//!   - internal/controller/protection/usage/reconciler.go
//!
//! A `Usage` declares that one resource (`spec.by`, the *using* resource) uses
//! another (`spec.of`, the *used* resource), and gates deletion of the `of`
//! resource while any `Usage` references it. Upstream enforces this with two
//! pieces:
//!   1. A reconciler that stamps the `crossplane.io/in-use: "true"` label and
//!      the `usage.apiextensions.crossplane.io` finalizer onto the `of`
//!      resource, and removes them once no `Usage` references it.
//!   2. A validating admission webhook that *denies* DELETE on any resource
//!      carrying the in-use label while a `Usage` exists, recording a
//!      deletion-attempt annotation so the delete can be *replayed* later if
//!      `spec.replayDeletion` is set.
//!
//! The admission *decision*, the in-use label policy, and the `replayDeletion`
//! planning are pure in-crate policy — exactly what this module ports and
//! tests. The only apiserver-coupled residual is the actual finalizer
//! write-back and webhook *registration* against a live apiserver, which
//! remains a thin cave-apiserver adapter (the [[skipped]]→[[mapped]] reversal
//! mirrors the composition-revision-garbage-collect reversal: a policy fully
//! ownable in-crate was previously mis-cut to a sibling).

use dashmap::DashMap;
use std::collections::HashMap;

/// Label key stamped onto the used (`of`) resource while it is referenced.
/// Upstream `inUseLabelKey`.
pub const IN_USE_LABEL: &str = "crossplane.io/in-use";

/// Finalizer protecting the `of` resource from deletion. Upstream `finalizer`.
pub const FINALIZER: &str = "usage.apiextensions.crossplane.io";

/// Annotation recording that a (denied) deletion of the `of` resource was
/// attempted, so the delete can be replayed when the Usage is removed.
/// Upstream `protection.AnnotationKeyDeletionAttempt`.
pub const DELETION_ATTEMPT_ANNOTATION: &str = "crossplane.io/deletion-attempt";

/// Identifies a Kubernetes resource by apiVersion/kind/name (+ optional
/// namespace). This is the *resolved* form of `spec.of` / `spec.by` — upstream
/// also supports a `resourceSelector.matchLabels` form whose resolution
/// requires listing live objects from the apiserver (cave-apiserver residual);
/// here we operate on the resolved reference, which is what the protection
/// decision ultimately compares.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceTarget {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

impl ResourceTarget {
    pub fn new(
        api_version: impl Into<String>,
        kind: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            api_version: api_version.into(),
            kind: kind.into(),
            name: name.into(),
            namespace: None,
        }
    }

    /// Scope the target to a namespace (for namespaced `of`/`by` resources).
    pub fn in_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }
}

/// A Usage resource: `of` is used by `by`, optionally with a human `reason`,
/// optionally replaying the deletion of `of` once the Usage is removed.
#[derive(Debug, Clone)]
pub struct Usage {
    pub name: String,
    pub of: ResourceTarget,
    pub by: Option<ResourceTarget>,
    pub reason: Option<String>,
    pub replay_deletion: bool,
}

impl Usage {
    pub fn new(name: impl Into<String>, of: ResourceTarget) -> Self {
        Self {
            name: name.into(),
            of,
            by: None,
            reason: None,
            replay_deletion: false,
        }
    }

    pub fn with_by(mut self, by: ResourceTarget) -> Self {
        self.by = Some(by);
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_replay_deletion(mut self, replay: bool) -> Self {
        self.replay_deletion = replay;
        self
    }
}

/// Outcome of the deletion-admission decision for a candidate `of` resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeletionDecision {
    /// No Usage references the resource — deletion may proceed.
    Allowed,
    /// At least one Usage references the resource — deletion is denied. Mirrors
    /// the admission webhook denial message listing the blocking usages.
    Denied {
        by_usages: Vec<String>,
        message: String,
    },
}

/// A planned replay of a previously-denied deletion (the resolved `of` target
/// of a removed Usage whose `replayDeletion` was set and whose `of` recorded a
/// deletion attempt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayDelete {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

/// In-memory store of Usages + the deletion-attempt ledger.
pub struct UsageStore {
    usages: DashMap<String, Usage>,
    /// Resources for which a (denied) deletion was attempted while protected.
    deletion_attempts: DashMap<ResourceTarget, ()>,
}

impl Default for UsageStore {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageStore {
    pub fn new() -> Self {
        Self {
            usages: DashMap::new(),
            deletion_attempts: DashMap::new(),
        }
    }

    /// The in-use label map applied to a protected `of` resource.
    pub fn in_use_label_map() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(IN_USE_LABEL.to_string(), "true".to_string());
        m
    }

    pub fn register(&self, usage: Usage) {
        self.usages.insert(usage.name.clone(), usage);
    }

    pub fn remove(&self, name: &str) -> Option<Usage> {
        self.usages.remove(name).map(|(_, u)| u)
    }

    pub fn len(&self) -> usize {
        self.usages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.usages.is_empty()
    }

    pub fn list(&self) -> Vec<Usage> {
        self.usages.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get(&self, name: &str) -> Option<Usage> {
        self.usages.get(name).map(|e| e.value().clone())
    }

    /// All Usages whose `of` resolves to `target`, returned in stable
    /// (name-sorted) order for deterministic admission messages.
    pub fn usages_of(&self, target: &ResourceTarget) -> Vec<Usage> {
        let mut out: Vec<Usage> = self
            .usages
            .iter()
            .filter(|e| &e.value().of == target)
            .map(|e| e.value().clone())
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Whether the resource is referenced by any Usage as its `of`.
    pub fn is_protected(&self, target: &ResourceTarget) -> bool {
        self.usages.iter().any(|e| &e.value().of == target)
    }

    /// The admission decision for deleting `target`. Denies while any Usage
    /// references it, recording a deletion-attempt (the annotation the webhook
    /// stamps on the `of` resource) so the delete can be replayed later.
    pub fn admit_deletion(&self, target: &ResourceTarget) -> DeletionDecision {
        let blocking = self.usages_of(target);
        if blocking.is_empty() {
            return DeletionDecision::Allowed;
        }
        // Record the attempt (idempotent) — mirrors the webhook annotating the
        // used resource with crossplane.io/deletion-attempt.
        self.deletion_attempts.insert(target.clone(), ());

        let by_usages: Vec<String> = blocking.iter().map(|u| u.name.clone()).collect();
        // Compose a denial message, surfacing the first explicit reason if any.
        let reason = blocking
            .iter()
            .find_map(|u| u.reason.clone())
            .unwrap_or_else(|| "the resource is in use".to_string());
        let message = format!(
            "deletion of {} {} denied: {} (blocked by usage(s): {})",
            target.kind,
            target.name,
            reason,
            by_usages.join(", "),
        );
        DeletionDecision::Denied { by_usages, message }
    }

    /// Whether a (denied) deletion attempt was recorded for `target`.
    pub fn had_deletion_attempt(&self, target: &ResourceTarget) -> bool {
        self.deletion_attempts.contains_key(target)
    }

    /// Plan the replay of `of`'s deletion when the named Usage is removed.
    ///
    /// Returns `Some` iff the Usage has `replayDeletion = true` *and* a deletion
    /// attempt was previously recorded for its `of` target — exactly the
    /// upstream guard (`replayDeletion && annotation present`).
    pub fn plan_replay(&self, usage_name: &str) -> Option<ReplayDelete> {
        let usage = self.get(usage_name)?;
        if !usage.replay_deletion {
            return None;
        }
        if !self.had_deletion_attempt(&usage.of) {
            return None;
        }
        Some(ReplayDelete {
            api_version: usage.of.api_version.clone(),
            kind: usage.of.kind.clone(),
            name: usage.of.name.clone(),
            namespace: usage.of.namespace.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip() {
        let s = UsageStore::new();
        assert!(s.is_empty());
        s.register(Usage::new("u", ResourceTarget::new("v1", "Secret", "s")));
        assert_eq!(s.len(), 1);
        assert!(s.get("u").is_some());
        assert!(s.remove("u").is_some());
        assert!(s.is_empty());
    }

    #[test]
    fn label_map_value_is_true() {
        let m = UsageStore::in_use_label_map();
        assert_eq!(m.get(IN_USE_LABEL).unwrap(), "true");
    }
}

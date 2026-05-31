// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composition selection — which Composition does a composite resource bind to?
//!
//! Upstream: crossplane/crossplane v2.3.1
//!   internal/controller/apiextensions/composite/api.go
//!     - APILabelSelectorResolver.SelectComposition
//!     - EnforcedCompositionSelector
//!     - APIDefaultCompositionSelector
//!
//! A composite resource (XR) chooses its Composition in one of three ways,
//! resolved in priority order:
//!   1. `spec.compositionRef` — an explicit name; selection is a no-op.
//!   2. `spec.compositionSelector.matchLabels` — pick a Composition whose
//!      labels match *and* whose `compositeTypeRef` is compatible with the XR.
//!   3. XRD `defaultCompositionRef` — a fallback used only when neither a ref
//!      nor a selector is set.
//! An XRD may also set `enforcedCompositionRef`, which overrides everything.
//!
//! This is pure in-crate policy with no apiserver coupling — the resolver
//! operates over the in-memory set of candidate Compositions.

use std::collections::BTreeMap;

/// A Composition as seen by the selector: its name, `metadata.labels`, and its
/// `spec.compositeTypeRef` (apiVersion + kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionCandidate {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub api_version: String,
    pub kind: String,
}

impl CompositionCandidate {
    /// Whether this candidate's composite type matches the XR's type — the
    /// upstream compatibility filter (`comp.Spec.CompositeTypeRef.APIVersion`
    /// and `.Kind`).
    fn is_compatible(&self, xr_api_version: &str, xr_kind: &str) -> bool {
        self.api_version == xr_api_version && self.kind == xr_kind
    }

    /// Whether this candidate's labels are a superset of `match_labels`.
    fn matches_labels(&self, match_labels: &BTreeMap<String, String>) -> bool {
        match_labels
            .iter()
            .all(|(k, v)| self.labels.get(k).map(|cv| cv == v).unwrap_or(false))
    }
}

/// Outcome of a label-selector resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// `spec.compositionRef` was already set — nothing to do.
    AlreadySet(String),
    /// A Composition was selected by label + type compatibility.
    Selected(String),
}

/// Why selection could not produce a Composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectError {
    /// No `compositionRef` and no `compositionSelector` were provided.
    NoSelector,
    /// No candidate matched both the label selector and the composite type
    /// (upstream `errNoCompatibleComposition`).
    NoCompatibleComposition,
}

/// Resolver for `spec.compositionSelector.matchLabels` (upstream
/// `APILabelSelectorResolver`).
pub struct LabelSelectorResolver;

impl LabelSelectorResolver {
    /// Resolve the Composition for an XR of type (`xr_api_version`, `xr_kind`).
    ///
    /// * If `current_ref` is `Some`, selection is a no-op
    ///   ([`SelectionOutcome::AlreadySet`]).
    /// * Otherwise `match_labels` must be present; candidates are filtered to
    ///   those matching every label *and* compatible with the XR's composite
    ///   type. If none remain, [`SelectError::NoCompatibleComposition`].
    /// * When several match, upstream picks at random; we pick the
    ///   lowest-named candidate for determinism (a deliberate, documented
    ///   divergence — random selection is non-reproducible and untestable).
    pub fn select(
        xr_api_version: &str,
        xr_kind: &str,
        current_ref: Option<&str>,
        match_labels: Option<&BTreeMap<String, String>>,
        candidates: &[CompositionCandidate],
    ) -> Result<SelectionOutcome, SelectError> {
        if let Some(name) = current_ref {
            return Ok(SelectionOutcome::AlreadySet(name.to_string()));
        }
        let match_labels = match_labels.ok_or(SelectError::NoSelector)?;

        let mut matching: Vec<&CompositionCandidate> = candidates
            .iter()
            .filter(|c| c.is_compatible(xr_api_version, xr_kind) && c.matches_labels(match_labels))
            .collect();
        if matching.is_empty() {
            return Err(SelectError::NoCompatibleComposition);
        }
        matching.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(SelectionOutcome::Selected(matching[0].name.clone()))
    }
}

/// Resolver for XRD `enforcedCompositionRef` (upstream
/// `EnforcedCompositionSelector`): when set, it overrides any existing ref.
pub struct EnforcedCompositionSelector;

impl EnforcedCompositionSelector {
    /// Returns the enforced composition name when the XRD enforces one. The
    /// caller overwrites the XR's `compositionRef` with this unconditionally.
    pub fn select(enforced_ref: Option<&str>) -> Option<String> {
        enforced_ref.map(|s| s.to_string())
    }
}

/// Resolver for XRD `defaultCompositionRef` (upstream
/// `APIDefaultCompositionSelector`): a fallback applied only when the XR has
/// neither a `compositionRef` nor a `compositionSelector`.
pub struct DefaultCompositionSelector;

impl DefaultCompositionSelector {
    pub fn select(
        default_ref: Option<&str>,
        current_ref: Option<&str>,
        has_selector: bool,
    ) -> Option<String> {
        if current_ref.is_some() || has_selector {
            return None;
        }
        default_ref.map(|s| s.to_string())
    }
}

/// `spec.compositionUpdatePolicy` — how an XR tracks Composition revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionUpdatePolicy {
    /// Always track the latest CompositionRevision.
    Automatic,
    /// Pin to an explicitly-chosen CompositionRevision.
    Manual,
}

impl CompositionUpdatePolicy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Automatic" => Some(Self::Automatic),
            "Manual" => Some(Self::Manual),
            _ => None,
        }
    }

    /// The effective CompositionRevision number for the policy:
    ///   * `Automatic` → `latest`,
    ///   * `Manual`    → `pinned` (falling back to `latest` if unpinned).
    pub fn effective_revision(&self, latest: u32, pinned: Option<u32>) -> u32 {
        match self {
            Self::Automatic => latest,
            Self::Manual => pinned.unwrap_or(latest),
        }
    }
}

impl Default for CompositionUpdatePolicy {
    /// Upstream defaults `compositionUpdatePolicy` to `Automatic`.
    fn default() -> Self {
        Self::Automatic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_automatic() {
        assert_eq!(CompositionUpdatePolicy::default(), CompositionUpdatePolicy::Automatic);
    }

    #[test]
    fn label_subset_match() {
        let c = CompositionCandidate {
            name: "c".into(),
            labels: [("a", "1"), ("b", "2")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            api_version: "g/v1".into(),
            kind: "K".into(),
        };
        let want: BTreeMap<String, String> =
            [("a", "1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert!(c.matches_labels(&want));
    }
}

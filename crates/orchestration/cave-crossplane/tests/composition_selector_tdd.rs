// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD coverage for composition selection (src/composition/selector.rs).
//!
//! Upstream: crossplane/crossplane v2.3.1
//!   internal/controller/apiextensions/composite/api.go
//!     - APILabelSelectorResolver.SelectComposition
//!     - EnforcedCompositionSelector
//!     - APIDefaultCompositionSelector
//!
//! Selection is pure in-crate policy: given an XR's compositionRef /
//! compositionSelector and the set of candidate Compositions, decide which
//! Composition the XR binds to. No apiserver coupling.

use std::collections::BTreeMap;

use cave_crossplane::composition::selector::{
    CompositionCandidate, CompositionUpdatePolicy, DefaultCompositionSelector,
    EnforcedCompositionSelector, LabelSelectorResolver, SelectError, SelectionOutcome,
};

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn candidate(name: &str, ls: &[(&str, &str)], api: &str, kind: &str) -> CompositionCandidate {
    CompositionCandidate {
        name: name.to_string(),
        labels: labels(ls),
        api_version: api.to_string(),
        kind: kind.to_string(),
    }
}

#[test]
fn no_op_when_composition_ref_already_set() {
    let cands = vec![candidate("c1", &[("env", "prod")], "example.org/v1", "XDatabase")];
    let sel = labels(&[("env", "prod")]);
    let out = LabelSelectorResolver::select(
        "example.org/v1",
        "XDatabase",
        Some("pinned"),
        Some(&sel),
        &cands,
    )
    .unwrap();
    assert_eq!(out, SelectionOutcome::AlreadySet("pinned".to_string()));
}

#[test]
fn selects_single_label_match() {
    let cands = vec![
        candidate("prod-comp", &[("env", "prod")], "example.org/v1", "XDatabase"),
        candidate("dev-comp", &[("env", "dev")], "example.org/v1", "XDatabase"),
    ];
    let sel = labels(&[("env", "prod")]);
    let out =
        LabelSelectorResolver::select("example.org/v1", "XDatabase", None, Some(&sel), &cands)
            .unwrap();
    assert_eq!(out, SelectionOutcome::Selected("prod-comp".to_string()));
}

#[test]
fn requires_all_match_labels() {
    let cands = vec![
        // Matches env but not tier → excluded.
        candidate("partial", &[("env", "prod")], "example.org/v1", "XDatabase"),
        candidate(
            "full",
            &[("env", "prod"), ("tier", "gold")],
            "example.org/v1",
            "XDatabase",
        ),
    ];
    let sel = labels(&[("env", "prod"), ("tier", "gold")]);
    let out =
        LabelSelectorResolver::select("example.org/v1", "XDatabase", None, Some(&sel), &cands)
            .unwrap();
    assert_eq!(out, SelectionOutcome::Selected("full".to_string()));
}

#[test]
fn filters_incompatible_composite_type() {
    // Label matches but the composite type is wrong → not a candidate.
    let cands = vec![
        candidate("wrong-kind", &[("env", "prod")], "example.org/v1", "XBucket"),
        candidate(
            "wrong-version",
            &[("env", "prod")],
            "example.org/v2",
            "XDatabase",
        ),
    ];
    let sel = labels(&[("env", "prod")]);
    let err =
        LabelSelectorResolver::select("example.org/v1", "XDatabase", None, Some(&sel), &cands)
            .unwrap_err();
    assert_eq!(err, SelectError::NoCompatibleComposition);
}

#[test]
fn error_when_no_label_match() {
    let cands = vec![candidate("c", &[("env", "dev")], "example.org/v1", "XDatabase")];
    let sel = labels(&[("env", "prod")]);
    let err =
        LabelSelectorResolver::select("example.org/v1", "XDatabase", None, Some(&sel), &cands)
            .unwrap_err();
    assert_eq!(err, SelectError::NoCompatibleComposition);
}

#[test]
fn error_when_no_selector_and_no_ref() {
    let cands = vec![candidate("c", &[("env", "prod")], "example.org/v1", "XDatabase")];
    let err =
        LabelSelectorResolver::select("example.org/v1", "XDatabase", None, None, &cands)
            .unwrap_err();
    assert_eq!(err, SelectError::NoSelector);
}

#[test]
fn deterministic_pick_lowest_name_when_multiple_match() {
    // Upstream picks at random; we pick the lowest name for determinism.
    let cands = vec![
        candidate("zeta", &[("env", "prod")], "example.org/v1", "XDatabase"),
        candidate("alpha", &[("env", "prod")], "example.org/v1", "XDatabase"),
        candidate("mid", &[("env", "prod")], "example.org/v1", "XDatabase"),
    ];
    let sel = labels(&[("env", "prod")]);
    let out =
        LabelSelectorResolver::select("example.org/v1", "XDatabase", None, Some(&sel), &cands)
            .unwrap();
    assert_eq!(out, SelectionOutcome::Selected("alpha".to_string()));
}

#[test]
fn enforced_overrides_any_existing_ref() {
    // XRD.enforcedCompositionRef set → always wins, overwriting current ref.
    assert_eq!(
        EnforcedCompositionSelector::select(Some("enforced")),
        Some("enforced".to_string())
    );
    assert_eq!(EnforcedCompositionSelector::select(None), None);
}

#[test]
fn default_applies_only_when_no_ref_and_no_selector() {
    // Default applies when neither ref nor selector is set.
    assert_eq!(
        DefaultCompositionSelector::select(Some("def"), None, false),
        Some("def".to_string())
    );
    // A pre-existing ref suppresses the default.
    assert_eq!(DefaultCompositionSelector::select(Some("def"), Some("x"), false), None);
    // A selector suppresses the default.
    assert_eq!(DefaultCompositionSelector::select(Some("def"), None, true), None);
    // No default configured → nothing.
    assert_eq!(DefaultCompositionSelector::select(None, None, false), None);
}

#[test]
fn update_policy_resolves_effective_revision() {
    assert_eq!(
        CompositionUpdatePolicy::from_str("Automatic"),
        Some(CompositionUpdatePolicy::Automatic)
    );
    assert_eq!(
        CompositionUpdatePolicy::from_str("Manual"),
        Some(CompositionUpdatePolicy::Manual)
    );
    assert_eq!(CompositionUpdatePolicy::from_str("nope"), None);

    // Automatic → always the latest revision.
    assert_eq!(
        CompositionUpdatePolicy::Automatic.effective_revision(7, Some(3)),
        7
    );
    // Manual → the pinned revision (falls back to latest if unpinned).
    assert_eq!(
        CompositionUpdatePolicy::Manual.effective_revision(7, Some(3)),
        3
    );
    assert_eq!(CompositionUpdatePolicy::Manual.effective_revision(7, None), 7);
}

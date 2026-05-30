// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of argocd-image-updater write-back — upstream
//! argoproj-labs/argocd-image-updater `pkg/argocd/update.go` +
//! `pkg/image/{options,version}.go`.
//!
//! The registry-poll daemon (watching container registries on a timer) stays
//! deferred; cave-deploy ports the **write-back computation**: parse the
//! `image-list` annotation, apply the `allow-tags` filter + update strategy
//! to a candidate tag set, and compute the Helm-parameter / Kustomize-image
//! overrides to commit back to git.

use cave_deploy::image_updater::{
    allow_tag, helm_writeback, kustomize_writeback, parse_image_list, select_tag, CandidateTag,
    UpdateStrategy,
};

// ─── strategy parsing (incl. deprecated aliases) ────────────────────────────

#[test]
fn update_strategy_parse_canonical_and_deprecated() {
    assert_eq!(UpdateStrategy::parse("semver"), Some(UpdateStrategy::Semver));
    assert_eq!(
        UpdateStrategy::parse("newest-build"),
        Some(UpdateStrategy::NewestBuild)
    );
    assert_eq!(
        UpdateStrategy::parse("alphabetical"),
        Some(UpdateStrategy::Alphabetical)
    );
    assert_eq!(UpdateStrategy::parse("digest"), Some(UpdateStrategy::Digest));
    // deprecated aliases
    assert_eq!(
        UpdateStrategy::parse("latest"),
        Some(UpdateStrategy::NewestBuild)
    );
    assert_eq!(
        UpdateStrategy::parse("name"),
        Some(UpdateStrategy::Alphabetical)
    );
    assert_eq!(UpdateStrategy::parse("bogus"), None);
}

// ─── image-list annotation parsing ──────────────────────────────────────────

#[test]
fn parse_image_list_alias_image_constraint() {
    let ann = "myalias=ghcr.io/acme/api:~1.2, web=nginx";
    let specs = parse_image_list(ann);
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].alias, "myalias");
    assert_eq!(specs[0].image_name, "ghcr.io/acme/api");
    assert_eq!(specs[0].constraint.as_deref(), Some("~1.2"));
    // no alias, no constraint
    assert_eq!(specs[1].alias, "web");
    assert_eq!(specs[1].image_name, "nginx");
    assert_eq!(specs[1].constraint, None);
}

#[test]
fn parse_image_list_without_alias_uses_image_as_alias() {
    let specs = parse_image_list("redis:6");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].alias, "redis");
    assert_eq!(specs[0].image_name, "redis");
    assert_eq!(specs[0].constraint.as_deref(), Some("6"));
}

// ─── allow-tags filter ──────────────────────────────────────────────────────

#[test]
fn allow_tag_regexp_and_any() {
    assert!(allow_tag(Some("regexp:^v?[0-9]+\\.[0-9]+\\.[0-9]+$"), "1.2.3"));
    assert!(allow_tag(Some("regexp:^v?[0-9]+\\.[0-9]+\\.[0-9]+$"), "v2.0.0"));
    assert!(!allow_tag(
        Some("regexp:^v?[0-9]+\\.[0-9]+\\.[0-9]+$"),
        "latest"
    ));
    // "any" and None accept everything
    assert!(allow_tag(Some("any"), "anything"));
    assert!(allow_tag(None, "whatever"));
}

// ─── tag selection per strategy ─────────────────────────────────────────────

fn cands(tags: &[(&str, i64)]) -> Vec<CandidateTag> {
    tags.iter()
        .map(|(n, c)| CandidateTag {
            name: n.to_string(),
            created: Some(*c),
        })
        .collect()
}

#[test]
fn select_tag_semver_respects_constraint() {
    let c = cands(&[("1.2.0", 1), ("1.2.5", 2), ("1.3.0", 3), ("2.0.0", 4)]);
    // ~1.2 → highest 1.2.x
    assert_eq!(
        select_tag(UpdateStrategy::Semver, &c, Some("~1.2.0"), None).as_deref(),
        Some("1.2.5")
    );
    // no constraint → highest valid semver
    assert_eq!(
        select_tag(UpdateStrategy::Semver, &c, None, None).as_deref(),
        Some("2.0.0")
    );
}

#[test]
fn select_tag_semver_with_allow_tags_filter() {
    let c = cands(&[("1.2.0", 1), ("1.3.0", 2), ("latest", 3)]);
    // allow-tags filters non-semver "latest" out (it would not be valid semver anyway)
    assert_eq!(
        select_tag(
            UpdateStrategy::Semver,
            &c,
            None,
            Some("regexp:^[0-9]")
        )
        .as_deref(),
        Some("1.3.0")
    );
}

#[test]
fn select_tag_newest_build_by_created_date() {
    let c = cands(&[("a", 100), ("b", 300), ("c", 200)]);
    assert_eq!(
        select_tag(UpdateStrategy::NewestBuild, &c, None, None).as_deref(),
        Some("b")
    );
}

#[test]
fn select_tag_alphabetical_picks_lexically_last() {
    let c = cands(&[("v1", 1), ("v3", 2), ("v2", 3)]);
    assert_eq!(
        select_tag(UpdateStrategy::Alphabetical, &c, None, None).as_deref(),
        Some("v3")
    );
}

#[test]
fn select_tag_empty_candidates_is_none() {
    assert_eq!(select_tag(UpdateStrategy::Semver, &[], None, None), None);
}

// ─── write-back computation ─────────────────────────────────────────────────

#[test]
fn helm_writeback_sets_name_and_tag_parameters() {
    let wb = helm_writeback("image.repository", "image.tag", "ghcr.io/acme/api", "1.3.0");
    assert_eq!(
        wb.helm_parameters,
        vec![
            ("image.repository".to_string(), "ghcr.io/acme/api".to_string()),
            ("image.tag".to_string(), "1.3.0".to_string()),
        ]
    );
    assert!(wb.kustomize_images.is_empty());
}

#[test]
fn kustomize_writeback_sets_image_override() {
    let wb = kustomize_writeback("nginx", "nginx", "1.25.3");
    assert_eq!(wb.kustomize_images, vec!["nginx=nginx:1.25.3".to_string()]);
    assert!(wb.helm_parameters.is_empty());
}

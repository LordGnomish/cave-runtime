// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Image Updater write-back — pure-Rust port of argoproj-labs/argocd-image-updater.
//!
//! Upstream: `pkg/image/options.go` (annotation parsing), `pkg/image/version.go`
//! (update strategies + tag filtering), `pkg/argocd/update.go` (Helm-parameter
//! and Kustomize-image write-back).
//!
//! The registry-poll daemon (the long-running controller that lists tags from
//! container registries on a timer) stays deferred — that is the operational
//! half that belongs in a dedicated runtime.  This module ports the
//! **write-back computation**:
//!
//!   * [`parse_image_list`] — the `image-list` annotation (`alias=image:constraint`)
//!   * [`UpdateStrategy`]    — `semver` / `newest-build` / `alphabetical` / `digest`
//!                            (plus deprecated `latest` / `name` aliases)
//!   * [`allow_tag`]         — the `allow-tags` `regexp:` / `any` filter
//!   * [`select_tag`]        — pick the new tag from a candidate set
//!   * [`helm_writeback`] /  — compute the parameter / image overrides to
//!     [`kustomize_writeback`]   commit back to git
//!
//! Given a candidate tag set (which a Phase-2 registry client would supply),
//! everything here is deterministic and dependency-light.

use crate::helm_deps::{max_satisfying, semver_satisfies};
use regex::Regex;

// ─── Update strategy ────────────────────────────────────────────────────────

/// Image Updater tag-selection strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStrategy {
    /// Highest allowed semantic version.
    Semver,
    /// Most recent creation date.
    NewestBuild,
    /// Latest alphabetically sorted tag.
    Alphabetical,
    /// Most recent pushed version of a mutable tag.
    Digest,
}

impl UpdateStrategy {
    /// Parse a strategy name, accepting the deprecated `latest` / `name`
    /// aliases for `newest-build` / `alphabetical`.
    pub fn parse(s: &str) -> Option<UpdateStrategy> {
        match s.trim() {
            "semver" => Some(UpdateStrategy::Semver),
            "newest-build" | "latest" => Some(UpdateStrategy::NewestBuild),
            "alphabetical" | "name" => Some(UpdateStrategy::Alphabetical),
            "digest" => Some(UpdateStrategy::Digest),
            _ => None,
        }
    }
}

// ─── image-list annotation ──────────────────────────────────────────────────

/// One entry of the `argocd-image-updater.argoproj.io/image-list` annotation.
#[derive(Debug, Clone)]
pub struct ImageSpec {
    /// Unique alias keying the per-image config annotations.
    pub alias: String,
    /// Full image name without the tag (`registry/repo`).
    pub image_name: String,
    /// Optional version constraint (the `:tail` of `image:constraint`).
    pub constraint: Option<String>,
}

/// Parse the `image-list` annotation: a comma-separated list of
/// `[alias=]image[:constraint]` entries.
pub fn parse_image_list(annotation: &str) -> Vec<ImageSpec> {
    annotation
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_one)
        .collect()
}

fn parse_one(entry: &str) -> ImageSpec {
    let (alias, image_part) = match entry.split_once('=') {
        Some((a, rest)) => (Some(a.trim().to_string()), rest.trim()),
        None => (None, entry),
    };
    // split image and constraint on the LAST ':' that is not part of a
    // registry host:port — image-updater treats the tail after ':' as the
    // version constraint, and registries here are host/repo so the only ':'
    // that matters is the tag separator on the final path segment.
    let (image_name, constraint) = split_image_constraint(image_part);
    let alias = alias.unwrap_or_else(|| {
        // derive alias from the last path component of the image name.
        image_name
            .rsplit('/')
            .next()
            .unwrap_or(&image_name)
            .to_string()
    });
    ImageSpec {
        alias,
        image_name,
        constraint,
    }
}

fn split_image_constraint(image_part: &str) -> (String, Option<String>) {
    // consider only the final path segment for a tag separator.
    match image_part.rsplit_once('/') {
        Some((prefix, last)) => match last.split_once(':') {
            Some((name, constraint)) => (
                format!("{}/{}", prefix, name),
                Some(constraint.to_string()),
            ),
            None => (image_part.to_string(), None),
        },
        None => match image_part.split_once(':') {
            Some((name, constraint)) => (name.to_string(), Some(constraint.to_string())),
            None => (image_part.to_string(), None),
        },
    }
}

// ─── allow-tags filter ──────────────────────────────────────────────────────

/// Does `tag` pass the `allow-tags` match function?  Supports `regexp:<expr>`
/// and `any` (the default); `None` accepts everything.
pub fn allow_tag(allow: Option<&str>, tag: &str) -> bool {
    match allow.map(str::trim) {
        None | Some("") | Some("any") => true,
        Some(spec) => match spec.strip_prefix("regexp:") {
            Some(pattern) => Regex::new(pattern).map(|re| re.is_match(tag)).unwrap_or(false),
            // a bare expression is treated as a regexp (image-updater default).
            None => Regex::new(spec).map(|re| re.is_match(tag)).unwrap_or(false),
        },
    }
}

// ─── tag selection ──────────────────────────────────────────────────────────

/// A candidate tag, optionally annotated with its registry build timestamp
/// (a Phase-2 registry client supplies `created`; `newest-build` needs it).
#[derive(Debug, Clone)]
pub struct CandidateTag {
    pub name: String,
    /// Unix-seconds creation timestamp, if known.
    pub created: Option<i64>,
}

/// Select the winning tag from `candidates` for `strategy`, after applying the
/// `allow_tags` filter and (for `semver`) the version `constraint`.
pub fn select_tag(
    strategy: UpdateStrategy,
    candidates: &[CandidateTag],
    constraint: Option<&str>,
    allow_tags: Option<&str>,
) -> Option<String> {
    let allowed: Vec<&CandidateTag> = candidates
        .iter()
        .filter(|c| allow_tag(allow_tags, &c.name))
        .collect();
    if allowed.is_empty() {
        return None;
    }
    match strategy {
        UpdateStrategy::Semver => {
            let constraint = constraint.unwrap_or("*");
            let names: Vec<String> = allowed
                .iter()
                .map(|c| c.name.clone())
                .filter(|n| semver_satisfies(n, constraint))
                .collect();
            max_satisfying(&names, constraint)
        }
        UpdateStrategy::NewestBuild => allowed
            .iter()
            .max_by_key(|c| c.created.unwrap_or(i64::MIN))
            .map(|c| c.name.clone()),
        UpdateStrategy::Alphabetical => {
            allowed.iter().map(|c| c.name.clone()).max()
        }
        UpdateStrategy::Digest => {
            // digest tracks a mutable tag — keep the configured constraint tag
            // if present among allowed, else the first allowed tag.
            match constraint {
                Some(t) if allowed.iter().any(|c| c.name == t) => Some(t.to_string()),
                _ => allowed.first().map(|c| c.name.clone()),
            }
        }
    }
}

// ─── write-back ─────────────────────────────────────────────────────────────

/// The overrides to commit back to git for an image update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteBack {
    /// Helm `--set NAME=VALUE` parameters (ordered: name then tag).
    pub helm_parameters: Vec<(String, String)>,
    /// Kustomize `images:` overrides (`original=newimage:tag`).
    pub kustomize_images: Vec<String>,
}

/// Helm write-back: set the image-name and image-tag parameters at the
/// configured dotted paths (`<alias>.helm.image-name` / `image-tag`).
pub fn helm_writeback(
    image_name_param: &str,
    image_tag_param: &str,
    repository: &str,
    tag: &str,
) -> WriteBack {
    WriteBack {
        helm_parameters: vec![
            (image_name_param.to_string(), repository.to_string()),
            (image_tag_param.to_string(), tag.to_string()),
        ],
        kustomize_images: Vec::new(),
    }
}

/// Kustomize write-back: emit an `images:` override mapping the original image
/// name to the new image and tag (`<alias>.kustomize.image-name`).
pub fn kustomize_writeback(original_name: &str, new_image: &str, tag: &str) -> WriteBack {
    WriteBack {
        helm_parameters: Vec::new(),
        kustomize_images: vec![format!("{}={}:{}", original_name, new_image, tag)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_handles_registry_with_port() {
        let (name, c) = split_image_constraint("registry.local:5000/acme/api:1.2");
        assert_eq!(name, "registry.local:5000/acme/api");
        assert_eq!(c.as_deref(), Some("1.2"));
    }
}

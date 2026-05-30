// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Faithful line-port of pulpcore/app/tasks/reclaim_space.py (pulpcore 3.49.0).
//
//! Disk-space reclamation planner — Pulp's `reclaim_space` task logic.
//!
//! Distinct from the orphan garbage collector in `src/core/gc.rs`: that sweeps
//! blobs nothing references. Reclamation instead targets content that IS still
//! referenced by repository versions but whose bytes can be safely dropped and
//! later re-downloaded on demand. Upstream `reclaim_space(repo_pks,
//! keeplist_rv_pks, force)`:
//!
//!   1. Collect the content of the targeted repository versions.
//!   2. Exclude content that appears in any keep-list repository version
//!      (`keeplist_rv_pks`) — those bytes must stay resident.
//!   3. Of the remainder, only content that has at least one `RemoteArtifact`
//!      (i.e. an upstream it can be re-fetched from) is reclaimable; immediate
//!      / uploaded content with no remote is preserved to avoid data loss.
//!   4. Delete the downloaded `Artifact` files for the reclaimable content,
//!      keeping the `ContentArtifact` metadata so the unit stays installable
//!      on-demand.
//!
//! This module ports steps 1–3 — the pure planning set-logic that decides
//! *which* content is reclaimable and how many bytes that frees. The actual
//! file deletion (step 4) is a storage-backend side effect owned by
//! cave-runtime's object store, intentionally out of scope here.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One (content, repository-version) membership row with the facts the planner
/// needs: whether the content has a remote (can be re-downloaded) and how many
/// bytes its downloaded artifact currently occupies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentArtifactRef {
    /// Stable identity of the content unit.
    pub content_pk: String,
    /// The repository version this membership belongs to.
    pub repo_version_pk: String,
    /// True when at least one `RemoteArtifact` exists for this content (it can
    /// be re-fetched on demand after its bytes are dropped).
    pub has_remote: bool,
    /// Bytes the downloaded artifact occupies (counted once per content).
    pub downloaded_bytes: u64,
}

/// Inputs to a reclamation run (port of the `reclaim_space` task arguments).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReclaimRequest {
    /// Repository versions to reclaim space from (`repo_pks` -> latest version).
    pub target_repo_versions: Vec<String>,
    /// Repository versions whose content must be kept resident (`keeplist`).
    pub keeplist_repo_versions: Vec<String>,
    /// Flat membership table for the relevant content.
    pub refs: Vec<ContentArtifactRef>,
}

/// Result of planning a reclamation run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReclaimPlan {
    /// Content units whose downloaded bytes can be dropped, sorted + unique.
    pub reclaimable_content: Vec<String>,
    /// Total bytes freed (counted once per content unit).
    pub reclaimed_bytes: u64,
}

/// Plan a reclamation run — the pure set-logic core of `reclaim_space`.
pub fn plan_reclaim(req: &ReclaimRequest) -> ReclaimPlan {
    let targets: BTreeSet<&str> = req
        .target_repo_versions
        .iter()
        .map(String::as_str)
        .collect();
    let keep: BTreeSet<&str> = req
        .keeplist_repo_versions
        .iter()
        .map(String::as_str)
        .collect();

    // Content protected by the keep-list: any content appearing in a keep-list
    // repository version is excluded from reclamation (step 2).
    let protected: BTreeSet<&str> = req
        .refs
        .iter()
        .filter(|r| keep.contains(r.repo_version_pk.as_str()))
        .map(|r| r.content_pk.as_str())
        .collect();

    // Walk the targeted-version memberships, collecting reclaimable content.
    // Bytes are counted once per content unit (the artifact is shared across
    // the versions that reference it).
    let mut bytes_by_content: BTreeMap<&str, u64> = BTreeMap::new();
    for r in &req.refs {
        if !targets.contains(r.repo_version_pk.as_str()) {
            continue; // step 1: only targeted versions
        }
        if protected.contains(r.content_pk.as_str()) {
            continue; // step 2: keep-list protected
        }
        if !r.has_remote {
            continue; // step 3: no remote -> not safely reclaimable
        }
        bytes_by_content
            .entry(r.content_pk.as_str())
            .or_insert(r.downloaded_bytes);
    }

    let reclaimed_bytes = bytes_by_content.values().sum();
    let reclaimable_content = bytes_by_content
        .keys()
        .map(|s| s.to_string())
        .collect();

    ReclaimPlan {
        reclaimable_content,
        reclaimed_bytes,
    }
}

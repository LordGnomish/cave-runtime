// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD RED (2026-05-30): port of pulpcore/app/tasks/reclaim_space.py —
// the disk-space reclamation planner. Distinct from the orphan GC already in
// src/core/gc.rs: reclaim_space targets STILL-REFERENCED, on-demand-capable
// content and drops its downloaded Artifact bytes while preserving metadata,
// skipping anything protected by a keep-list of repository versions.
//
// Drives a NOT-YET-EXISTING module `cave_artifacts::pulp::reclaim`.
//
// Upstream reference (pulpcore 3.49.0):
//   - pulpcore/app/tasks/reclaim_space.py :: reclaim_space()

use cave_artifacts::pulp::reclaim::{plan_reclaim, ContentArtifactRef, ReclaimRequest};

fn cref(content: &str, repo_version: &str, on_demand_remote: bool, size: u64) -> ContentArtifactRef {
    ContentArtifactRef {
        content_pk: content.to_string(),
        repo_version_pk: repo_version.to_string(),
        has_remote: on_demand_remote,
        downloaded_bytes: size,
    }
}

#[test]
fn reclaims_on_demand_content_in_targeted_repo() {
    // One content in repo-version rvA, has a remote, currently downloaded.
    let req = ReclaimRequest {
        target_repo_versions: vec!["rvA".into()],
        keeplist_repo_versions: vec![],
        refs: vec![cref("c1", "rvA", true, 500)],
    };
    let plan = plan_reclaim(&req);
    assert_eq!(plan.reclaimable_content, vec!["c1".to_string()]);
    assert_eq!(plan.reclaimed_bytes, 500);
}

#[test]
fn skips_content_without_remote() {
    // Immediate (uploaded) content has no remote -> cannot be re-fetched, so
    // it is never reclaimed (would be data loss).
    let req = ReclaimRequest {
        target_repo_versions: vec!["rvA".into()],
        keeplist_repo_versions: vec![],
        refs: vec![cref("c1", "rvA", false, 500)],
    };
    let plan = plan_reclaim(&req);
    assert!(plan.reclaimable_content.is_empty());
    assert_eq!(plan.reclaimed_bytes, 0);
}

#[test]
fn skips_content_protected_by_keeplist() {
    // c1 is in both the targeted rvA and the keep-list rvK -> protected.
    let req = ReclaimRequest {
        target_repo_versions: vec!["rvA".into()],
        keeplist_repo_versions: vec!["rvK".into()],
        refs: vec![
            cref("c1", "rvA", true, 500),
            cref("c1", "rvK", true, 500),
        ],
    };
    let plan = plan_reclaim(&req);
    assert!(plan.reclaimable_content.is_empty());
    assert_eq!(plan.reclaimed_bytes, 0);
}

#[test]
fn ignores_content_outside_target_repo_versions() {
    // c2 lives only in rvB which is not targeted -> untouched.
    let req = ReclaimRequest {
        target_repo_versions: vec!["rvA".into()],
        keeplist_repo_versions: vec![],
        refs: vec![
            cref("c1", "rvA", true, 100),
            cref("c2", "rvB", true, 999),
        ],
    };
    let plan = plan_reclaim(&req);
    assert_eq!(plan.reclaimable_content, vec!["c1".to_string()]);
    assert_eq!(plan.reclaimed_bytes, 100);
}

#[test]
fn deduplicates_content_present_in_multiple_targeted_versions() {
    // Same content c1 in two targeted versions: reclaim once, count bytes once.
    let req = ReclaimRequest {
        target_repo_versions: vec!["rvA".into(), "rvA2".into()],
        keeplist_repo_versions: vec![],
        refs: vec![
            cref("c1", "rvA", true, 700),
            cref("c1", "rvA2", true, 700),
        ],
    };
    let plan = plan_reclaim(&req);
    assert_eq!(plan.reclaimable_content, vec!["c1".to_string()]);
    assert_eq!(plan.reclaimed_bytes, 700);
}

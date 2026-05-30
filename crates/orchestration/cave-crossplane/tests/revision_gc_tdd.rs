// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: composition/package revisionHistoryLimit garbage collection.
//!
//! Upstream: crossplane/crossplane v2.3.1
//!   internal/controller/pkg/manager/reconciler.go (GC block)
//!
//! Closes the [[skipped]] `composition-revision-garbage-collect`. The prior
//! retention was a hardcoded `MAX_REVISIONS = 10` drain ring; upstream GC is:
//!   * configurable revisionHistoryLimit (`nil` → default 1, `0` → keep all /
//!     GC disabled, `n` → keep current + n historical),
//!   * keeps the current (highest-numbered / active) revision always,
//!   * collects the OLDEST (lowest-numbered) revision(s) when
//!     `len(revisions) > limit + 1`.

use cave_crossplane::composition::revision_gc::RevisionGarbageCollector as Gc;
use cave_crossplane::composition::CompositionStore;
use cave_crossplane::models::{CompositionMode, CreateCompositionRequest, TypeRef};

fn req(name: &str) -> CreateCompositionRequest {
    CreateCompositionRequest {
        name: name.into(),
        composite_type_ref: TypeRef {
            api_version: "ex.cave.io/v1".into(),
            kind: "XDb".into(),
        },
        resources: vec![],
        pipeline: vec![],
        mode: CompositionMode::Pipeline,
        patch_sets: vec![],
    }
}

// ── plan(): pure GC algorithm ───────────────────────────────────────────────

#[test]
fn default_limit_keeps_current_plus_one() {
    // nil limit → default 1 → keep current(5) + 1 historical(4); GC 1,2,3.
    let revs = vec![1, 2, 3, 4, 5];
    let gc = Gc::plan(&revs, 5, None);
    assert_eq!(gc, vec![1, 2, 3], "default limit 1 keeps current + 1 historical");
}

#[test]
fn limit_zero_disables_gc() {
    let revs = vec![1, 2, 3, 4, 5];
    assert!(Gc::plan(&revs, 5, Some(0)).is_empty(), "limit 0 must keep all revisions");
}

#[test]
fn limit_three_keeps_current_plus_three() {
    let revs = vec![1, 2, 3, 4, 5, 6];
    // keep current(6) + 3 historical (3,4,5) → GC 1,2.
    assert_eq!(Gc::plan(&revs, 6, Some(3)), vec![1, 2]);
}

#[test]
fn no_gc_when_at_or_below_limit_plus_one() {
    // limit 3 → threshold len > 4; here len == 4 → no GC.
    let revs = vec![1, 2, 3, 4];
    assert!(Gc::plan(&revs, 4, Some(3)).is_empty());
}

#[test]
fn never_collects_current_revision() {
    // current is the lowest-numbered (pathological) — must still be preserved.
    let revs = vec![3, 4, 5];
    let gc = Gc::plan(&revs, 3, Some(0)); // disabled anyway
    assert!(!gc.contains(&3));
    // And with an aggressive limit the current is still excluded.
    let gc2 = Gc::plan(&revs, 3, Some(1));
    assert!(!gc2.contains(&3), "current revision must never be garbage-collected");
}

#[test]
fn plan_one_returns_single_oldest() {
    // Faithful per-reconcile behaviour: upstream deletes ONE oldest per pass.
    let revs = vec![1, 2, 3, 4, 5];
    assert_eq!(Gc::plan_one(&revs, 5, Some(1)), Some(1));
    // At limit+1 → nothing to collect this pass.
    assert_eq!(Gc::plan_one(&[4, 5], 5, Some(1)), None);
}

// ── CompositionStore integration ────────────────────────────────────────────

#[test]
fn store_gc_revisions_respects_limit() {
    let s = CompositionStore::new();
    let mut c = s.create(req("c1")).unwrap();
    // Build revisions 2..=12 (create() seeded revision 1).
    for r in 2..=12u32 {
        c.revision = r;
        s.push_revision("c1", c.clone());
    }
    // 12 revisions on disk. GC down to current(12) + 2 historical.
    let collected = s.gc_revisions("c1", Some(2)).unwrap();
    let remaining = s.get_revisions("c1").unwrap();
    assert_eq!(remaining.len(), 3, "keep current + 2 historical");
    assert_eq!(collected, 9, "12 - 3 collected");
    let nums: Vec<u32> = remaining.iter().map(|r| r.revision).collect();
    assert_eq!(nums, vec![10, 11, 12], "newest revisions retained");
}

#[test]
fn store_gc_disabled_keeps_all() {
    let s = CompositionStore::new();
    let mut c = s.create(req("c1")).unwrap();
    for r in 2..=8u32 {
        c.revision = r;
        s.push_revision("c1", c.clone());
    }
    let collected = s.gc_revisions("c1", Some(0)).unwrap();
    assert_eq!(collected, 0);
    assert_eq!(s.get_revisions("c1").unwrap().len(), 8);
}

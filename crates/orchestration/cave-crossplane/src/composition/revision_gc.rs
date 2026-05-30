// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CompositionRevision / PackageRevision garbage collection.
//!
//! Upstream: crossplane/crossplane v2.3.1
//!   internal/controller/pkg/manager/reconciler.go (GC block)
//!
//! Replaces the prior hardcoded `MAX_REVISIONS = 10` drain ring with the real
//! upstream `revisionHistoryLimit` policy:
//!   * `nil`     → default limit (1),
//!   * `Some(0)` → GC disabled, keep all revisions,
//!   * `Some(n)` → keep the current revision + `n` historical revisions.
//!
//! The current (highest-numbered / active) revision is *always* preserved, and
//! garbage collection removes the OLDEST (lowest-numbered) revision(s) once
//! `len(revisions) > limit + 1` — exactly mirroring upstream, where the `+1`
//! reserves a slot for the active revision on top of the history allowance.

/// Garbage collector for revision history bounded by `revisionHistoryLimit`.
pub struct RevisionGarbageCollector;

impl RevisionGarbageCollector {
    /// Default `revisionHistoryLimit` when the spec leaves it unset.
    /// Upstream `apis/pkg/v1` default for package revision history is `1`.
    pub const DEFAULT_HISTORY_LIMIT: i64 = 1;

    /// Effective limit: `nil` falls back to the default.
    fn effective_limit(limit: Option<i64>) -> i64 {
        limit.unwrap_or(Self::DEFAULT_HISTORY_LIMIT)
    }

    /// Plan the full set of revision numbers eligible for garbage collection,
    /// oldest-first. Steady-state convergence equivalent of running the
    /// per-reconcile [`plan_one`](Self::plan_one) until stable.
    ///
    /// Returns an empty plan when GC is disabled (`Some(0)`) or the history is
    /// already within `limit + 1`. The `current` revision is never collected.
    pub fn plan(revisions: &[u32], current: u32, limit: Option<i64>) -> Vec<u32> {
        let limit = Self::effective_limit(limit);
        // limit == 0 → GC disabled, keep all. (Upstream guards `!= 0`.)
        if limit == 0 {
            return Vec::new();
        }
        let keep = (limit + 1) as usize; // current + `limit` historical
        if revisions.len() <= keep {
            return Vec::new();
        }
        // Candidates = every revision except the current, sorted oldest-first.
        let mut candidates: Vec<u32> = revisions
            .iter()
            .copied()
            .filter(|&r| r != current)
            .collect();
        candidates.sort_unstable();
        let to_collect = revisions.len() - keep;
        candidates.truncate(to_collect);
        candidates
    }

    /// Faithful per-reconcile step: collect the single OLDEST revision when
    /// over the limit, else `None`. Upstream deletes one revision per
    /// reconcile pass (and requeues) rather than batch-deleting.
    pub fn plan_one(revisions: &[u32], current: u32, limit: Option<i64>) -> Option<u32> {
        Self::plan(revisions, current, limit).into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_one() {
        assert_eq!(RevisionGarbageCollector::DEFAULT_HISTORY_LIMIT, 1);
        // nil → default 1 → keep current + 1.
        assert_eq!(RevisionGarbageCollector::plan(&[1, 2, 3], 3, None), vec![1]);
    }

    #[test]
    fn disabled_keeps_all() {
        assert!(RevisionGarbageCollector::plan(&[1, 2, 3, 4], 4, Some(0)).is_empty());
    }

    #[test]
    fn empty_history() {
        assert!(RevisionGarbageCollector::plan(&[], 0, Some(1)).is_empty());
    }
}

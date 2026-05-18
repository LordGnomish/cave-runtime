// SPDX-License-Identifier: AGPL-3.0-or-later
//! Multi-tenant primitives + retention/compaction policy types for cave-logs.
//!
//! The existing `LogStore` already keys chunks by tenant, but Loki-parity
//! requires the `tenant_id` label to *also* appear on the stream so that
//! LogQL queries can filter by `{tenant_id="acme"}` directly. This module
//! provides those helpers and a structured `RetentionPolicy` /
//! `CompactionPolicy` pair the runtime can compose against the store.

use crate::models::{Labels, LogEntry, TimestampNs};
use std::collections::HashMap;
use std::time::Duration;

pub const TENANT_LABEL: &str = "tenant_id";
pub const DEFAULT_TENANT: &str = "anonymous";
pub const X_SCOPE_ORG_ID: &str = "X-Scope-OrgID";

// ─── Header parsing ────────────────────────────────────────────────────────

pub fn tenant_from_headers(headers: &HashMap<String, String>) -> String {
    for (k, v) in headers.iter() {
        if k.eq_ignore_ascii_case(X_SCOPE_ORG_ID) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    DEFAULT_TENANT.to_string()
}

// ─── Stream-label injection ────────────────────────────────────────────────

/// Add `tenant_id={tenant}` to a stream's label-set. Idempotent (overwrites
/// any spoofed value coming in through user input).
pub fn inject_tenant_stream_label(labels: &mut Labels, tenant: &str) {
    labels.0.insert(TENANT_LABEL.to_string(), tenant.to_string());
}

/// Normalize a stream label-set: ensure `tenant_id` exists *and* equals
/// the authorized tenant. Returns the label-set unchanged if already correct.
pub fn normalize_tenant_label(labels: &Labels, tenant: &str) -> Labels {
    let mut copy = labels.clone();
    inject_tenant_stream_label(&mut copy, tenant);
    copy
}

// ─── Retention ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    /// Default retention for streams that don't match any per-stream rule.
    pub default: Duration,
    /// Per-stream-label-match overrides — first match wins.
    pub overrides: Vec<RetentionOverride>,
}

#[derive(Debug, Clone)]
pub struct RetentionOverride {
    pub label: String,
    pub value: String,
    pub retention: Duration,
}

impl RetentionPolicy {
    pub fn new(default: Duration) -> Self {
        RetentionPolicy { default, overrides: Vec::new() }
    }

    pub fn with_override(mut self, label: impl Into<String>, value: impl Into<String>, d: Duration) -> Self {
        self.overrides.push(RetentionOverride {
            label: label.into(),
            value: value.into(),
            retention: d,
        });
        self
    }

    pub fn for_stream(&self, labels: &Labels) -> Duration {
        for o in &self.overrides {
            if labels.0.get(&o.label).map(|v| v.as_str()) == Some(o.value.as_str()) {
                return o.retention;
            }
        }
        self.default
    }

    /// Cutoff timestamp (ns) for entries belonging to this stream.
    /// Anything strictly older than the cutoff is eligible for deletion.
    pub fn cutoff_ns(&self, labels: &Labels, now_ns: TimestampNs) -> TimestampNs {
        let d = self.for_stream(labels);
        let nanos = d.as_nanos();
        // Cap at i64::MAX so casting can't wrap, and clamp the result at 0
        // (TimestampNs is signed but we don't expose negative cutoffs).
        let to_subtract = if nanos > i64::MAX as u128 { i64::MAX } else { nanos as i64 };
        now_ns.saturating_sub(to_subtract).max(0)
    }
}

#[derive(Debug, Clone)]
pub struct RetentionPlan {
    /// Number of entries that *would* be deleted on the next prune.
    pub deletable_entries: usize,
    /// Total number of entries inspected.
    pub inspected_entries: usize,
}

/// Dry-run pruning — counts how many entries would be removed without
/// touching anything.
pub fn dry_run_retention(
    labels: &Labels,
    entries: &[LogEntry],
    policy: &RetentionPolicy,
    now_ns: TimestampNs,
) -> RetentionPlan {
    let cutoff = policy.cutoff_ns(labels, now_ns);
    let deletable = entries.iter().filter(|e| e.ts < cutoff).count();
    RetentionPlan {
        deletable_entries: deletable,
        inspected_entries: entries.len(),
    }
}

// ─── Compaction ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    /// Minimum number of chunks per stream before compaction is considered.
    pub min_chunks: usize,
    /// Don't compact a chunk above this size (it's already big enough).
    pub max_chunk_bytes: usize,
    /// Compact eagerly once a stream's small-chunk count crosses this.
    pub small_chunk_count_trigger: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        CompactionPolicy {
            min_chunks: 4,
            max_chunk_bytes: 1024 * 1024, // 1 MiB
            small_chunk_count_trigger: 16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPlan {
    /// Indices into the input slice that should be merged together.
    pub merge_groups: Vec<Vec<usize>>,
    pub total_compactable_bytes: usize,
}

/// Plan a compaction: greedily group consecutive small chunks into
/// merge candidates of `target_bytes`, leaving large chunks alone.
pub fn plan_compaction(
    chunk_sizes: &[usize],
    target_bytes: usize,
    policy: &CompactionPolicy,
) -> CompactionPlan {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_size: usize = 0;
    let mut total_compactable = 0usize;

    for (i, &size) in chunk_sizes.iter().enumerate() {
        if size > policy.max_chunk_bytes {
            // Already large enough — flush whatever we were grouping
            if current.len() >= policy.min_chunks {
                groups.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            current_size = 0;
            continue;
        }
        current.push(i);
        current_size += size;
        total_compactable += size;
        if current_size >= target_bytes {
            groups.push(std::mem::take(&mut current));
            current_size = 0;
        }
    }
    if current.len() >= policy.min_chunks {
        groups.push(current);
    }

    CompactionPlan { merge_groups: groups, total_compactable_bytes: total_compactable }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Labels;
    use std::collections::HashMap;
    use std::time::Duration;

    fn lbl(pairs: &[(&str, &str)]) -> Labels {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.to_string());
        }
        Labels(m)
    }

    fn entry(ts: TimestampNs, line: &str) -> LogEntry {
        LogEntry { ts, line: line.into(), metadata: HashMap::new() }
    }

    // ─── tenant_from_headers ─────────────────────────────────────────────

    #[test]
    fn test_tenant_default_when_missing() {
        assert_eq!(tenant_from_headers(&HashMap::new()), DEFAULT_TENANT);
    }

    #[test]
    fn test_tenant_from_canonical_header() {
        let mut h = HashMap::new();
        h.insert(X_SCOPE_ORG_ID.into(), "acme".into());
        assert_eq!(tenant_from_headers(&h), "acme");
    }

    #[test]
    fn test_tenant_lookup_case_insensitive() {
        let mut h = HashMap::new();
        h.insert("x-scope-orgid".into(), "acme".into());
        assert_eq!(tenant_from_headers(&h), "acme");
    }

    #[test]
    fn test_tenant_blank_falls_back() {
        let mut h = HashMap::new();
        h.insert(X_SCOPE_ORG_ID.into(), "  ".into());
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    // ─── inject_tenant_stream_label ──────────────────────────────────────

    #[test]
    fn test_inject_adds_label() {
        let mut l = lbl(&[("app", "api")]);
        inject_tenant_stream_label(&mut l, "acme");
        assert_eq!(l.0.get(TENANT_LABEL), Some(&"acme".to_string()));
    }

    #[test]
    fn test_inject_overwrites_spoofed_value() {
        let mut l = lbl(&[(TENANT_LABEL, "spoof"), ("app", "api")]);
        inject_tenant_stream_label(&mut l, "acme");
        assert_eq!(l.0.get(TENANT_LABEL), Some(&"acme".to_string()));
    }

    #[test]
    fn test_normalize_tenant_label_returns_owned() {
        let l = lbl(&[("app", "api")]);
        let out = normalize_tenant_label(&l, "acme");
        assert_eq!(out.0.get(TENANT_LABEL), Some(&"acme".to_string()));
        // Original unchanged
        assert!(l.0.get(TENANT_LABEL).is_none());
    }

    // ─── RetentionPolicy ─────────────────────────────────────────────────

    #[test]
    fn test_policy_default_for_unmatched_stream() {
        let p = RetentionPolicy::new(Duration::from_secs(3600));
        assert_eq!(p.for_stream(&lbl(&[("app", "x")])), Duration::from_secs(3600));
    }

    #[test]
    fn test_policy_override_first_match_wins() {
        let p = RetentionPolicy::new(Duration::from_secs(3600))
            .with_override("env", "prod", Duration::from_secs(86_400))
            .with_override("env", "prod", Duration::from_secs(99));
        assert_eq!(
            p.for_stream(&lbl(&[("env", "prod")])),
            Duration::from_secs(86_400)
        );
    }

    #[test]
    fn test_policy_override_stream_specific() {
        let p = RetentionPolicy::new(Duration::from_secs(3600))
            .with_override("env", "prod", Duration::from_secs(86_400));
        assert_eq!(
            p.for_stream(&lbl(&[("env", "stage")])),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn test_policy_cutoff_ns_subtracts_retention() {
        let p = RetentionPolicy::new(Duration::from_nanos(1000));
        let cutoff = p.cutoff_ns(&lbl(&[]), 5000);
        assert_eq!(cutoff, 4000);
    }

    #[test]
    fn test_policy_cutoff_ns_saturates_at_zero() {
        let p = RetentionPolicy::new(Duration::from_nanos(10_000));
        let cutoff = p.cutoff_ns(&lbl(&[]), 50);
        assert_eq!(cutoff, 0);
    }

    // ─── dry_run_retention ───────────────────────────────────────────────

    #[test]
    fn test_dry_run_counts_old_entries() {
        let p = RetentionPolicy::new(Duration::from_nanos(100));
        let now = 1000;
        let entries = vec![
            entry(800, "drop-1"),  // 800 < cutoff 900
            entry(850, "drop-2"),
            entry(900, "keep-1"),  // 900 not < 900
            entry(950, "keep-2"),
        ];
        let plan = dry_run_retention(&lbl(&[]), &entries, &p, now);
        assert_eq!(plan.deletable_entries, 2);
        assert_eq!(plan.inspected_entries, 4);
    }

    #[test]
    fn test_dry_run_zero_retention_keeps_only_now() {
        let p = RetentionPolicy::new(Duration::from_nanos(0));
        let now = 1000;
        let entries = vec![entry(999, "old"), entry(1000, "now")];
        let plan = dry_run_retention(&lbl(&[]), &entries, &p, now);
        // Cutoff = 1000; 999 < 1000 → deletable, 1000 < 1000 → not deletable
        assert_eq!(plan.deletable_entries, 1);
    }

    #[test]
    fn test_dry_run_per_stream_overrides() {
        let p = RetentionPolicy::new(Duration::from_secs(86_400 * 30)) // 30 days default
            .with_override("env", "ephemeral", Duration::from_nanos(100));
        let now = 10_000;
        let labels = lbl(&[("env", "ephemeral")]);
        let entries = vec![entry(0, "a"), entry(9_899, "b"), entry(9_900, "c")];
        let plan = dry_run_retention(&labels, &entries, &p, now);
        // cutoff = 9_900; entries < 9_900 → 2 deletable
        assert_eq!(plan.deletable_entries, 2);
    }

    // ─── CompactionPolicy ────────────────────────────────────────────────

    #[test]
    fn test_compaction_default() {
        let c = CompactionPolicy::default();
        assert_eq!(c.min_chunks, 4);
        assert_eq!(c.max_chunk_bytes, 1024 * 1024);
    }

    #[test]
    fn test_plan_compaction_groups_small_chunks() {
        let p = CompactionPolicy::default();
        let sizes = vec![100, 100, 100, 100, 100, 100, 100, 100, 100, 100];
        let target = 400;
        let plan = plan_compaction(&sizes, target, &p);
        // Greedy: groups of 4 chunks at 400 bytes = 5 indices, then 4 more
        // → 100+100+100+100 = 400 (group), 100+100+100+100 = 400 (group), 2 left below min_chunks
        assert!(!plan.merge_groups.is_empty());
        assert!(plan.merge_groups.iter().all(|g| !g.is_empty()));
        assert_eq!(plan.total_compactable_bytes, 1000);
    }

    #[test]
    fn test_plan_compaction_skips_already_large_chunks() {
        let p = CompactionPolicy { min_chunks: 2, max_chunk_bytes: 50, small_chunk_count_trigger: 8 };
        let sizes = vec![10, 10, 1000, 10, 10];
        let plan = plan_compaction(&sizes, 50, &p);
        // The 1000-byte chunk isn't a candidate; first pair {0,1} is OK
        // and last pair {3,4} forms another group.
        assert_eq!(plan.merge_groups.len(), 2);
        assert_eq!(plan.merge_groups[0], vec![0, 1]);
        assert_eq!(plan.merge_groups[1], vec![3, 4]);
        assert_eq!(plan.total_compactable_bytes, 40);
    }

    #[test]
    fn test_plan_compaction_below_min_chunks_drops_group() {
        let p = CompactionPolicy { min_chunks: 5, max_chunk_bytes: 1024, small_chunk_count_trigger: 16 };
        let sizes = vec![10, 10, 10];
        let plan = plan_compaction(&sizes, 50, &p);
        // Only 3 chunks but min_chunks=5 → no group
        assert!(plan.merge_groups.is_empty());
    }

    #[test]
    fn test_plan_compaction_target_split_into_chunks() {
        let p = CompactionPolicy { min_chunks: 1, max_chunk_bytes: 1024, small_chunk_count_trigger: 16 };
        let sizes = vec![100, 100, 100, 100];
        let plan = plan_compaction(&sizes, 200, &p);
        // 100+100=200 → flush; 100+100=200 → flush
        assert_eq!(plan.merge_groups.len(), 2);
    }

    #[test]
    fn test_plan_compaction_empty_input() {
        let p = CompactionPolicy::default();
        let plan = plan_compaction(&[], 100, &p);
        assert!(plan.merge_groups.is_empty());
        assert_eq!(plan.total_compactable_bytes, 0);
    }

    #[test]
    fn test_plan_compaction_single_oversize_chunk_no_groups() {
        let p = CompactionPolicy { min_chunks: 1, max_chunk_bytes: 50, small_chunk_count_trigger: 16 };
        let sizes = vec![1000];
        let plan = plan_compaction(&sizes, 200, &p);
        assert!(plan.merge_groups.is_empty());
        assert_eq!(plan.total_compactable_bytes, 0);
    }

    // ─── End-to-end-style integration ────────────────────────────────────

    #[test]
    fn test_round_trip_inject_then_normalize_idempotent() {
        let mut l = lbl(&[("app", "api")]);
        inject_tenant_stream_label(&mut l, "acme");
        let norm = normalize_tenant_label(&l, "acme");
        assert_eq!(norm.0.get(TENANT_LABEL), Some(&"acme".to_string()));
        assert_eq!(l.0, norm.0);
    }

    #[test]
    fn test_constants_match_loki_conventions() {
        assert_eq!(TENANT_LABEL, "tenant_id");
        assert_eq!(DEFAULT_TENANT, "anonymous");
        assert_eq!(X_SCOPE_ORG_ID, "X-Scope-OrgID");
    }

    #[test]
    fn test_per_stream_retention_isolation() {
        let p = RetentionPolicy::new(Duration::from_nanos(1000))
            .with_override("env", "prod", Duration::from_nanos(100))
            .with_override("env", "stage", Duration::from_nanos(10_000));
        assert_eq!(p.for_stream(&lbl(&[("env", "prod")])), Duration::from_nanos(100));
        assert_eq!(p.for_stream(&lbl(&[("env", "stage")])), Duration::from_nanos(10_000));
        assert_eq!(p.for_stream(&lbl(&[("env", "qa")])), Duration::from_nanos(1000));
    }

    #[test]
    fn test_dry_run_no_entries_zero_plan() {
        let p = RetentionPolicy::new(Duration::from_nanos(100));
        let plan = dry_run_retention(&lbl(&[]), &[], &p, 1000);
        assert_eq!(plan.deletable_entries, 0);
        assert_eq!(plan.inspected_entries, 0);
    }

    #[test]
    fn test_compaction_plan_total_excludes_oversized() {
        let p = CompactionPolicy { min_chunks: 1, max_chunk_bytes: 100, small_chunk_count_trigger: 8 };
        let sizes = vec![50, 50, 200, 50, 50];
        let plan = plan_compaction(&sizes, 100, &p);
        // 200 is excluded; small ones sum to 200
        assert_eq!(plan.total_compactable_bytes, 200);
    }

    #[test]
    fn test_inject_preserves_other_labels() {
        let mut l = lbl(&[("app", "api"), ("env", "prod"), ("region", "us")]);
        inject_tenant_stream_label(&mut l, "acme");
        assert_eq!(l.0.get("app"), Some(&"api".to_string()));
        assert_eq!(l.0.get("env"), Some(&"prod".to_string()));
        assert_eq!(l.0.get("region"), Some(&"us".to_string()));
        assert_eq!(l.0.get(TENANT_LABEL), Some(&"acme".to_string()));
    }
}

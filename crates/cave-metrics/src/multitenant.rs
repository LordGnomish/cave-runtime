//! Multi-tenant primitives for cave-metrics.
//!
//! - `tenant_from_headers` parses `X-Scope-OrgID` (Cortex/Mimir convention).
//! - `inject_tenant_label` stamps `tenant_id={tenant}` onto every label-set
//!   on ingest, so the TSDB physically separates series.
//! - `enforce_tenant_filter` ensures every query/federation matcher carries
//!   a `tenant_id=` matcher (added if absent), preventing cross-tenant reads.
//! - `federation_relabel` implements Prometheus's `honor_labels` semantics:
//!     - honor_labels=true  → keep source labels on conflict.
//!     - honor_labels=false → overwrite conflicting labels with `external`.
//!
//! Plus tenant cardinality helpers used by the `cardinality` API.

use crate::model::{LabelMatcher, Labels, MatchOp};
use std::collections::{HashMap, HashSet};

pub const TENANT_LABEL: &str = "tenant_id";
pub const DEFAULT_TENANT: &str = "anonymous";
pub const X_SCOPE_ORG_ID: &str = "X-Scope-OrgID";

/// Pull `X-Scope-OrgID` from a header map (case-insensitive).
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

/// Stamp `tenant_id={tenant}` onto a labels object, leaving other labels intact.
pub fn inject_tenant_label(labels: &mut Labels, tenant: &str) {
    labels.insert(TENANT_LABEL, tenant);
}

/// Add a `tenant_id=` matcher to the slice if it is not already present.
/// Returns the (possibly extended) matcher slice as an owned Vec.
pub fn enforce_tenant_filter(mut matchers: Vec<LabelMatcher>, tenant: &str) -> Vec<LabelMatcher> {
    let already = matchers.iter().any(|m| m.name == TENANT_LABEL);
    if !already {
        matchers.push(LabelMatcher::equal(TENANT_LABEL, tenant));
    }
    matchers
}

/// Reject any query that asks for a different tenant than the one
/// authorized by `X-Scope-OrgID`.
pub fn matches_tenant(matchers: &[LabelMatcher], allowed_tenant: &str) -> bool {
    for m in matchers {
        if m.name == TENANT_LABEL && m.op == MatchOp::Equal && m.value != allowed_tenant {
            return false;
        }
    }
    true
}

/// Apply Prometheus federation `honor_labels` semantics to a single
/// scraped series.
///
/// `external` is the label-set the federating Prom adds to *all* series
/// (e.g. `{job="federate"}`). When `honor_labels=true`, conflicting keys
/// are kept from the source; when false, they are overwritten by the
/// external value (Prometheus's default behaviour).
pub fn federation_relabel(source: &Labels, external: &Labels, honor_labels: bool) -> Labels {
    let mut merged = source.clone();
    for (k, v) in external.iter() {
        match (merged.get(k).is_some(), honor_labels) {
            (true, true) => {} // keep source
            _ => {
                merged.insert(k.to_string(), v.to_string());
            }
        }
    }
    merged
}

// ─── Cardinality helpers ───────────────────────────────────────────────────

/// Count distinct tenants currently emitting any series in `all_labels`.
pub fn tenant_count(all_labels: impl Iterator<Item = Labels>) -> usize {
    let mut seen = HashSet::new();
    for l in all_labels {
        if let Some(t) = l.get(TENANT_LABEL) {
            seen.insert(t.to_string());
        }
    }
    seen.len()
}

/// Series count per tenant.
pub fn series_per_tenant(all_labels: impl Iterator<Item = Labels>) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for l in all_labels {
        let t = l.get(TENANT_LABEL).unwrap_or(DEFAULT_TENANT).to_string();
        *out.entry(t).or_insert(0) += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Labels;
    use std::collections::HashMap;

    fn lbl(pairs: &[(&str, &str)]) -> Labels {
        Labels::from_pairs(pairs.iter().map(|(k, v)| (*k, *v)))
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
        h.insert(X_SCOPE_ORG_ID.into(), "   ".into());
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    #[test]
    fn test_tenant_empty_falls_back() {
        let mut h = HashMap::new();
        h.insert(X_SCOPE_ORG_ID.into(), "".into());
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    // ─── inject_tenant_label ─────────────────────────────────────────────

    #[test]
    fn test_inject_tenant_adds_label() {
        let mut l = lbl(&[("job", "api")]);
        inject_tenant_label(&mut l, "acme");
        assert_eq!(l.get(TENANT_LABEL), Some("acme"));
        assert_eq!(l.get("job"), Some("api"));
    }

    #[test]
    fn test_inject_tenant_overwrites_existing() {
        let mut l = lbl(&[(TENANT_LABEL, "old"), ("job", "api")]);
        inject_tenant_label(&mut l, "new");
        assert_eq!(l.get(TENANT_LABEL), Some("new"));
    }

    // ─── enforce_tenant_filter ──────────────────────────────────────────

    #[test]
    fn test_enforce_adds_when_missing() {
        let m = enforce_tenant_filter(vec![LabelMatcher::equal("__name__", "x")], "acme");
        assert_eq!(m.len(), 2);
        let t = m.iter().find(|m| m.name == TENANT_LABEL).unwrap();
        assert_eq!(t.value, "acme");
        assert_eq!(t.op, MatchOp::Equal);
    }

    #[test]
    fn test_enforce_skips_when_already_present() {
        let m = enforce_tenant_filter(
            vec![
                LabelMatcher::equal("__name__", "x"),
                LabelMatcher::equal(TENANT_LABEL, "globex"),
            ],
            "acme",
        );
        assert_eq!(m.len(), 2);
        // The user-supplied tenant matcher is preserved (caller must verify
        // it via matches_tenant() below)
        let t = m.iter().find(|m| m.name == TENANT_LABEL).unwrap();
        assert_eq!(t.value, "globex");
    }

    // ─── matches_tenant ──────────────────────────────────────────────────

    #[test]
    fn test_matches_tenant_when_no_matcher() {
        assert!(matches_tenant(&[LabelMatcher::equal("__name__", "x")], "acme"));
    }

    #[test]
    fn test_matches_tenant_when_equal() {
        let m = vec![LabelMatcher::equal(TENANT_LABEL, "acme")];
        assert!(matches_tenant(&m, "acme"));
    }

    #[test]
    fn test_matches_tenant_rejects_cross_tenant_query() {
        let m = vec![LabelMatcher::equal(TENANT_LABEL, "globex")];
        assert!(!matches_tenant(&m, "acme"));
    }

    // ─── federation_relabel ─────────────────────────────────────────────

    #[test]
    fn test_relabel_external_wins_when_no_honor() {
        let src = lbl(&[("job", "src"), ("instance", "i1")]);
        let ext = lbl(&[("job", "federate")]);
        let out = federation_relabel(&src, &ext, false);
        assert_eq!(out.get("job"), Some("federate"));
        assert_eq!(out.get("instance"), Some("i1"));
    }

    #[test]
    fn test_relabel_source_wins_when_honor_labels() {
        let src = lbl(&[("job", "src"), ("instance", "i1")]);
        let ext = lbl(&[("job", "federate")]);
        let out = federation_relabel(&src, &ext, true);
        assert_eq!(out.get("job"), Some("src"));
        assert_eq!(out.get("instance"), Some("i1"));
    }

    #[test]
    fn test_relabel_adds_external_when_no_conflict() {
        let src = lbl(&[("instance", "i1")]);
        let ext = lbl(&[("region", "us")]);
        let out = federation_relabel(&src, &ext, false);
        assert_eq!(out.get("region"), Some("us"));
        assert_eq!(out.get("instance"), Some("i1"));
    }

    #[test]
    fn test_relabel_honor_labels_still_adds_new_external() {
        let src = lbl(&[("instance", "i1")]);
        let ext = lbl(&[("region", "us")]);
        let out = federation_relabel(&src, &ext, true);
        assert_eq!(out.get("region"), Some("us"));
    }

    #[test]
    fn test_relabel_with_empty_external_is_passthrough() {
        let src = lbl(&[("a", "1")]);
        let out = federation_relabel(&src, &Labels::default(), false);
        assert_eq!(out.get("a"), Some("1"));
        assert_eq!(out.0.len(), 1);
    }

    // ─── tenant_count / series_per_tenant ────────────────────────────────

    #[test]
    fn test_tenant_count_distinct() {
        let all = vec![
            lbl(&[(TENANT_LABEL, "acme")]),
            lbl(&[(TENANT_LABEL, "globex")]),
            lbl(&[(TENANT_LABEL, "acme")]),
        ];
        assert_eq!(tenant_count(all.into_iter()), 2);
    }

    #[test]
    fn test_tenant_count_handles_missing_label() {
        let all = vec![
            lbl(&[(TENANT_LABEL, "acme")]),
            lbl(&[("__name__", "x")]),
        ];
        assert_eq!(tenant_count(all.into_iter()), 1);
    }

    #[test]
    fn test_series_per_tenant_counts() {
        let all = vec![
            lbl(&[(TENANT_LABEL, "acme")]),
            lbl(&[(TENANT_LABEL, "acme")]),
            lbl(&[(TENANT_LABEL, "globex")]),
            lbl(&[("__name__", "x")]),
        ];
        let map = series_per_tenant(all.into_iter());
        assert_eq!(map.get("acme"), Some(&2));
        assert_eq!(map.get("globex"), Some(&1));
        assert_eq!(map.get(DEFAULT_TENANT), Some(&1));
    }

    // ─── Integration: ingest + query round-trip semantics ────────────────

    #[test]
    fn test_ingest_then_filter_isolates_tenants() {
        // Two tenants with the same metric name
        let mut acme_labels = lbl(&[("__name__", "cpu"), ("instance", "a")]);
        let mut globex_labels = lbl(&[("__name__", "cpu"), ("instance", "g")]);
        inject_tenant_label(&mut acme_labels, "acme");
        inject_tenant_label(&mut globex_labels, "globex");
        let all = vec![acme_labels.clone(), globex_labels.clone()];

        let acme_filter = enforce_tenant_filter(vec![LabelMatcher::equal("__name__", "cpu")], "acme");
        let acme_visible: Vec<_> = all.iter()
            .filter(|l| acme_filter.iter().all(|m| m.matches(l)))
            .collect();
        assert_eq!(acme_visible.len(), 1);
        assert_eq!(acme_visible[0].get("instance"), Some("a"));
    }

    #[test]
    fn test_relabel_then_inject_tenant_idempotent() {
        let src = lbl(&[("job", "src")]);
        let ext = lbl(&[("region", "us")]);
        let mut out = federation_relabel(&src, &ext, true);
        inject_tenant_label(&mut out, "acme");
        // Idempotency: injecting twice still yields one tenant_id label
        inject_tenant_label(&mut out, "acme");
        assert_eq!(out.iter().filter(|(k, _)| *k == TENANT_LABEL).count(), 1);
    }

    #[test]
    fn test_default_tenant_constant_value() {
        assert_eq!(DEFAULT_TENANT, "anonymous");
    }

    #[test]
    fn test_x_scope_org_id_constant_value() {
        assert_eq!(X_SCOPE_ORG_ID, "X-Scope-OrgID");
    }

    #[test]
    fn test_tenant_label_constant_value() {
        assert_eq!(TENANT_LABEL, "tenant_id");
    }

    #[test]
    fn test_relabel_preserves_source_label_count_when_honor() {
        let src = lbl(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let ext = lbl(&[("a", "X"), ("b", "Y")]);
        let out = federation_relabel(&src, &ext, true);
        assert_eq!(out.0.len(), 3);
        assert_eq!(out.get("a"), Some("1"));
        assert_eq!(out.get("c"), Some("3"));
    }

    #[test]
    fn test_enforce_tenant_filter_preserves_other_matchers_order() {
        let m = enforce_tenant_filter(
            vec![
                LabelMatcher::equal("__name__", "cpu"),
                LabelMatcher::equal("job", "api"),
            ],
            "acme",
        );
        assert_eq!(m[0].name, "__name__");
        assert_eq!(m[1].name, "job");
        assert_eq!(m[2].name, TENANT_LABEL);
    }

    #[test]
    fn test_relabel_empty_source_uses_external() {
        let src = Labels::default();
        let ext = lbl(&[("a", "1")]);
        let out = federation_relabel(&src, &ext, false);
        assert_eq!(out.get("a"), Some("1"));
    }

    #[test]
    fn test_relabel_empty_source_honor_labels_uses_external() {
        // honor_labels only matters for *conflicts*; empty source has no conflicts.
        let src = Labels::default();
        let ext = lbl(&[("a", "1")]);
        let out = federation_relabel(&src, &ext, true);
        assert_eq!(out.get("a"), Some("1"));
    }
}

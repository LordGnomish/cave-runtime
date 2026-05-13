//! Global search — top-bar full-text index over admin routes,
//! resources, commands, and crate names.
//!
//! Distinct from [`crate::admin::search`] (OpenSearch-parity index
//! browser). This module owns the *navigation* surface — the
//! always-on input that fuzzy-matches across the Portal's known
//! routes / resources.
//!
//! Built once at startup from the admin router's known route list +
//! AdminState fixture content. The index is in-memory and supports
//! exact / prefix / substring / fuzzy match.
//!
//! Persona scope: PlatformAdmin sees everything; TenantAdmin sees
//! routes that are tenant-agnostic AND resources whose `tenant`
//! field matches the caller.

use crate::admin::permission::{Permission, Persona, RequestCtx};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GlobalSearchError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocKind {
    /// `/admin/*` page URL.
    Route,
    /// In-state resource (KEDA scaled object, vault path, ADR id).
    Resource,
    /// cavectl command name.
    Command,
    /// Crate name from the compliance dashboard.
    Crate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalSearchDoc {
    pub kind: DocKind,
    pub label: String,
    /// URL fragment a click should navigate to.
    pub href: String,
    /// Free-form body indexed alongside the label.
    pub body: String,
    /// Optional tenant scope — `None` ⇒ globally visible.
    pub tenant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GlobalSearchHit {
    pub doc: GlobalSearchDoc,
    /// Higher = better. Composed of signals: exact (5.0), prefix
    /// (3.0), substring (1.5), body-substring (1.0), fuzzy (≤ 0.5).
    pub score: f64,
}

#[derive(Debug)]
pub struct GlobalSearchIndex {
    docs: RwLock<Vec<GlobalSearchDoc>>,
}

impl GlobalSearchIndex {
    pub fn new() -> Self {
        Self {
            docs: RwLock::new(Vec::new()),
        }
    }

    pub fn add(&self, doc: GlobalSearchDoc) {
        self.docs.write().unwrap().push(doc);
    }

    pub fn add_many(&self, docs: impl IntoIterator<Item = GlobalSearchDoc>) {
        let mut g = self.docs.write().unwrap();
        for d in docs {
            g.push(d);
        }
    }

    pub fn len(&self) -> usize {
        self.docs.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Run a query against the index. `limit = 0` is unlimited.
    pub fn query(
        &self,
        ctx: &RequestCtx,
        q: &str,
        limit: usize,
    ) -> Result<Vec<GlobalSearchHit>, GlobalSearchError> {
        ctx.authorise(Permission::GlobalSearchRead)?;
        let q = q.trim();
        let q_lower = q.to_ascii_lowercase();
        let docs = self.docs.read().unwrap();
        let mut hits: Vec<GlobalSearchHit> = docs
            .iter()
            .filter(|d| visible_to(d, ctx))
            .filter_map(|d| {
                score_doc(d, q, &q_lower).map(|s| GlobalSearchHit { doc: d.clone(), score: s })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.doc.label.cmp(&b.doc.label))
        });
        if limit > 0 && hits.len() > limit {
            hits.truncate(limit);
        }
        Ok(hits)
    }
}

impl Default for GlobalSearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn visible_to(doc: &GlobalSearchDoc, ctx: &RequestCtx) -> bool {
    match &doc.tenant {
        None => true,
        Some(t) => ctx.persona == Persona::PlatformAdmin || ctx.tenant.as_str() == t,
    }
}

fn score_doc(doc: &GlobalSearchDoc, q: &str, q_lower: &str) -> Option<f64> {
    if q.is_empty() {
        return None;
    }
    let label_lower = doc.label.to_ascii_lowercase();
    let body_lower = doc.body.to_ascii_lowercase();
    let mut score = 0.0;
    if label_lower == q_lower {
        score += 5.0;
    } else if label_lower.starts_with(q_lower) {
        score += 3.0;
    } else if label_lower.contains(q_lower) {
        score += 1.5;
    }
    if body_lower.contains(q_lower) {
        score += 1.0;
    }
    if score == 0.0 {
        let dist = levenshtein(&label_lower, q_lower);
        let max = label_lower.len().max(q_lower.len()).max(1);
        let ratio = 1.0 - (dist as f64 / max as f64);
        if ratio >= 0.6 {
            score = ratio * 0.5;
        } else {
            return None;
        }
    }
    Some(score)
}

/// Plain iterative Levenshtein. Cheap for short label/query pairs.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0usize; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_platform() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::GlobalSearchRead])
    }

    fn ctx_tenant(tenant: &str) -> RequestCtx {
        RequestCtx::developer_as(
            tenant,
            &[Permission::GlobalSearchRead],
            Persona::TenantAdmin,
        )
    }

    fn seeded() -> GlobalSearchIndex {
        let idx = GlobalSearchIndex::new();
        idx.add_many([
            GlobalSearchDoc {
                kind: DocKind::Route,
                label: "compliance".into(),
                href: "/admin/compliance".into(),
                body: "charter compliance dashboard".into(),
                tenant: None,
            },
            GlobalSearchDoc {
                kind: DocKind::Route,
                label: "vault secrets".into(),
                href: "/admin/vault".into(),
                body: "vault secret browser".into(),
                tenant: None,
            },
            GlobalSearchDoc {
                kind: DocKind::Resource,
                label: "echo-scaler".into(),
                href: "/admin/keda/scaledobjects/echo-scaler".into(),
                body: "scaledobject".into(),
                tenant: Some("acme".into()),
            },
            GlobalSearchDoc {
                kind: DocKind::Resource,
                label: "billing-scaler".into(),
                href: "/admin/keda/scaledobjects/billing-scaler".into(),
                body: "scaledobject".into(),
                tenant: Some("other".into()),
            },
            GlobalSearchDoc {
                kind: DocKind::Crate,
                label: "cave-cache".into(),
                href: "/admin/compliance/cave-cache".into(),
                body: "Redis 7.2 reimplementation".into(),
                tenant: None,
            },
        ]);
        idx
    }

    #[test]
    fn exact_match_scores_highest() {
        let idx = seeded();
        let hits = idx.query(&ctx_platform(), "compliance", 10).unwrap();
        assert_eq!(hits[0].doc.label, "compliance");
        assert!(hits[0].score >= 5.0);
    }

    #[test]
    fn prefix_match_returned_before_substring() {
        let idx = seeded();
        let hits = idx.query(&ctx_platform(), "vault", 10).unwrap();
        assert_eq!(hits[0].doc.label, "vault secrets");
    }

    #[test]
    fn body_substring_match_returns_hit() {
        let idx = seeded();
        let hits = idx.query(&ctx_platform(), "redis", 10).unwrap();
        assert!(hits.iter().any(|h| h.doc.label == "cave-cache"));
    }

    #[test]
    fn fuzzy_match_handles_typo() {
        let idx = seeded();
        let hits = idx.query(&ctx_platform(), "complience", 10).unwrap();
        assert!(hits.iter().any(|h| h.doc.label == "compliance"));
    }

    #[test]
    fn fuzzy_does_not_match_random_strings() {
        let idx = seeded();
        assert!(idx.query(&ctx_platform(), "xyzzyqq", 10).unwrap().is_empty());
    }

    #[test]
    fn tenant_admin_sees_only_own_tenant_resources() {
        let idx = seeded();
        let hits = idx.query(&ctx_tenant("acme"), "scaler", 10).unwrap();
        let labels: Vec<&str> = hits.iter().map(|h| h.doc.label.as_str()).collect();
        assert!(labels.contains(&"echo-scaler"));
        assert!(!labels.contains(&"billing-scaler"));
    }

    #[test]
    fn platform_admin_sees_every_tenant() {
        let idx = seeded();
        let hits = idx.query(&ctx_platform(), "scaler", 10).unwrap();
        let labels: Vec<&str> = hits.iter().map(|h| h.doc.label.as_str()).collect();
        assert!(labels.contains(&"echo-scaler"));
        assert!(labels.contains(&"billing-scaler"));
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let idx = seeded();
        assert!(idx.query(&ctx_platform(), "", 10).unwrap().is_empty());
        assert!(idx.query(&ctx_platform(), "   ", 10).unwrap().is_empty());
    }

    #[test]
    fn limit_truncates_to_top_n() {
        let idx = GlobalSearchIndex::new();
        for i in 0..10 {
            idx.add(GlobalSearchDoc {
                kind: DocKind::Route,
                label: format!("route-{i}"),
                href: format!("/r/{i}"),
                body: "shared".into(),
                tenant: None,
            });
        }
        let hits = idx.query(&ctx_platform(), "route", 3).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn query_refuses_without_permission() {
        let idx = seeded();
        let ctx = RequestCtx::developer("acme", &[]);
        assert!(matches!(idx.query(&ctx, "x", 1).unwrap_err(), GlobalSearchError::Auth(_)));
    }

    #[test]
    fn levenshtein_known_pairs() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", "abc"), 0);
    }
}

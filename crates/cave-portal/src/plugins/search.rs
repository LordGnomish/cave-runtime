// SPDX-License-Identifier: AGPL-3.0-or-later
//! Search plugin — global cross-domain search.
//!
//! Indexes catalog entries, dashboards, ADRs, secrets metadata, deployments,
//! etc. into one weighted ranked index. Tenant scoped — results are filtered
//! by the caller's tenant id before ranking.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Service,
    Dashboard,
    Adr,
    SecretMeta,
    Deployment,
    Page,
    Runbook,
    Job,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedDoc {
    pub id: String,
    pub tenant: String,
    pub kind: DocKind,
    pub title: String,
    pub body: String,
    pub link: String,
    pub tags: Vec<String>,
    pub boost: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub doc: IndexedDoc,
    pub score: f64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SearchError {
    #[error("empty query")]
    EmptyQuery,
    #[error("query too long (max 200)")]
    TooLong,
    #[error("limit too large (max 100)")]
    LimitTooLarge,
}

const MAX_QUERY_LEN: usize = 200;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Default)]
pub struct SearchPlugin {
    docs: Vec<IndexedDoc>,
}

impl SearchPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn index(&mut self, doc: IndexedDoc) {
        if let Some(idx) = self
            .docs
            .iter()
            .position(|x| x.tenant == doc.tenant && x.id == doc.id && x.kind == doc.kind)
        {
            self.docs[idx] = doc;
        } else {
            self.docs.push(doc);
        }
    }

    pub fn remove(&mut self, tenant: &str, kind: DocKind, id: &str) -> bool {
        if let Some(idx) = self
            .docs
            .iter()
            .position(|x| x.tenant == tenant && x.kind == kind && x.id == id)
        {
            self.docs.remove(idx);
            true
        } else {
            false
        }
    }

    pub fn count(&self) -> usize {
        self.docs.len()
    }

    /// Default tenant query — case-insensitive substring matching plus
    /// per-field weighting (title 4×, tag 3×, body 1×). The `boost` field on
    /// the doc multiplies the final score.
    pub fn search(
        &self,
        tenant: &str,
        query: &str,
        kinds: &[DocKind],
        limit: usize,
    ) -> Result<Vec<SearchHit>, SearchError> {
        if query.is_empty() {
            return Err(SearchError::EmptyQuery);
        }
        if query.len() > MAX_QUERY_LEN {
            return Err(SearchError::TooLong);
        }
        if limit > MAX_LIMIT {
            return Err(SearchError::LimitTooLarge);
        }
        let q_lc = query.to_lowercase();
        let mut hits: Vec<SearchHit> = self
            .docs
            .iter()
            .filter(|d| d.tenant == tenant)
            .filter(|d| kinds.is_empty() || kinds.contains(&d.kind))
            .filter_map(|d| {
                let title_match = d.title.to_lowercase().contains(&q_lc);
                let body_match = d.body.to_lowercase().contains(&q_lc);
                let tag_match = d.tags.iter().any(|t| t.to_lowercase().contains(&q_lc));
                let mut score = 0.0;
                if title_match {
                    score += 4.0;
                }
                if tag_match {
                    score += 3.0;
                }
                if body_match {
                    score += 1.0;
                }
                if score == 0.0 {
                    return None;
                }
                let boosted = score * (1.0 + d.boost as f64 / 10.0);
                Some(SearchHit { doc: d.clone(), score: boosted })
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, kind: DocKind, title: &str, body: &str) -> IndexedDoc {
        IndexedDoc {
            id: id.into(),
            tenant: "acme".into(),
            kind,
            title: title.into(),
            body: body.into(),
            link: format!("/{id}"),
            tags: Vec::new(),
            boost: 0,
        }
    }

    #[test]
    fn index_inserts() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "T", ""));
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn index_replaces_same_key() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "Old", ""));
        p.index(doc("a", DocKind::Service, "New", ""));
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn index_separate_kinds_dont_collide() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "T", ""));
        p.index(doc("a", DocKind::Dashboard, "T", ""));
        assert_eq!(p.count(), 2);
    }

    #[test]
    fn remove_returns_true_when_present() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "T", ""));
        assert!(p.remove("acme", DocKind::Service, "a"));
        assert_eq!(p.count(), 0);
    }

    #[test]
    fn remove_returns_false_when_absent() {
        let mut p = SearchPlugin::new();
        assert!(!p.remove("acme", DocKind::Service, "ghost"));
    }

    #[test]
    fn search_empty_query_errors() {
        let p = SearchPlugin::new();
        let err = p.search("acme", "", &[], 10).unwrap_err();
        assert_eq!(err, SearchError::EmptyQuery);
    }

    #[test]
    fn search_too_long_errors() {
        let p = SearchPlugin::new();
        let q = "a".repeat(MAX_QUERY_LEN + 1);
        let err = p.search("acme", &q, &[], 10).unwrap_err();
        assert_eq!(err, SearchError::TooLong);
    }

    #[test]
    fn search_limit_too_large_errors() {
        let p = SearchPlugin::new();
        let err = p.search("acme", "x", &[], MAX_LIMIT + 1).unwrap_err();
        assert_eq!(err, SearchError::LimitTooLarge);
    }

    #[test]
    fn search_finds_in_title() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "Auth Service", ""));
        let hits = p.search("acme", "auth", &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_title_outweighs_body() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "Auth", "deploy notes"));
        p.index(doc("b", DocKind::Service, "Other", "auth notes"));
        let hits = p.search("acme", "auth", &[], 10).unwrap();
        assert_eq!(hits[0].doc.id, "a");
    }

    #[test]
    fn search_tag_match_boosts_score() {
        let mut p = SearchPlugin::new();
        let mut d = doc("a", DocKind::Service, "S", "x");
        d.tags = vec!["security".into()];
        p.index(d);
        let hits = p.search("acme", "security", &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score >= 3.0);
    }

    #[test]
    fn search_filters_by_kind() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "Auth", ""));
        p.index(doc("b", DocKind::Dashboard, "Auth", ""));
        let hits = p.search("acme", "auth", &[DocKind::Service], 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc.kind, DocKind::Service);
    }

    #[test]
    fn search_kinds_empty_means_all() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "Auth", ""));
        p.index(doc("b", DocKind::Dashboard, "Auth", ""));
        let hits = p.search("acme", "auth", &[], 10).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_filters_by_tenant() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "Auth", ""));
        let mut globex = doc("b", DocKind::Service, "Auth", "");
        globex.tenant = "globex".into();
        p.index(globex);
        let hits = p.search("acme", "auth", &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_respects_limit() {
        let mut p = SearchPlugin::new();
        for i in 0..20 {
            p.index(doc(&format!("d{i}"), DocKind::Service, "Auth Foo", ""));
        }
        let hits = p.search("acme", "auth", &[], 5).unwrap();
        assert_eq!(hits.len(), 5);
    }

    #[test]
    fn search_case_insensitive() {
        let mut p = SearchPlugin::new();
        p.index(doc("a", DocKind::Service, "AuthN", ""));
        let hits = p.search("acme", "AUTH", &[], 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_returns_descending_score() {
        let mut p = SearchPlugin::new();
        let mut a = doc("a", DocKind::Service, "Auth", "auth body");
        a.tags = vec!["auth".into()];
        p.index(a);
        p.index(doc("b", DocKind::Service, "X", "auth"));
        let hits = p.search("acme", "auth", &[], 10).unwrap();
        assert!(hits[0].score >= hits[1].score);
    }

    #[test]
    fn search_boost_multiplies_score() {
        let mut p = SearchPlugin::new();
        let mut a = doc("a", DocKind::Service, "Auth", "");
        a.boost = 0;
        p.index(a);
        let mut b = doc("b", DocKind::Service, "Auth", "");
        b.boost = 50;
        p.index(b);
        let hits = p.search("acme", "auth", &[], 10).unwrap();
        let a_score = hits.iter().find(|h| h.doc.id == "a").unwrap().score;
        let b_score = hits.iter().find(|h| h.doc.id == "b").unwrap().score;
        assert!(b_score > a_score);
    }
}

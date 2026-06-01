// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rerank endpoint (`POST /v1/rerank`) — Cohere/Jina-compatible cross-encoder
//! relevance scoring.
//!
//! Mirrors LiteLLM's `rerank_api/main.py` request/response contract (which in
//! turn follows Cohere's `/v1/rerank`). The relevance scoring itself is done
//! by an in-process lexical cross-encoder surrogate ([`lexical_rerank`]) so the
//! endpoint works without a remote rerank vendor; remote rerank backends
//! (Cohere/Jina) remain a Phase-2 provider-matrix concern.

use serde::{Deserialize, Serialize};

/// `POST /v1/rerank` request — Cohere/Jina-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankRequest {
    pub model: String,
    pub query: String,
    pub documents: Vec<String>,
    /// Return only the top-N results (after scoring). `None` = all documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<usize>,
    /// Echo the document text back inside each result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_documents: Option<bool>,
}

/// One reranked document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResult {
    /// Index into the original `documents` array.
    pub index: usize,
    /// Relevance score in `[0, 1]`, higher = more relevant.
    pub relevance_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<RerankDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankDocument {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankBilledUnits {
    pub search_units: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankMeta {
    pub billed_units: RerankBilledUnits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResponse {
    pub id: String,
    pub results: Vec<RerankResult>,
    pub model: String,
    pub meta: RerankMeta,
}

// ── Local lexical cross-encoder surrogate ──────────────────────────────────────

/// Lowercase, split on non-alphanumeric boundaries, drop empties.
fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// Deterministic FNV-1a 64-bit hash, used for a stable response `id`.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Score `documents` against `query` with a BM25 lexical relevance model and
/// return `(original_index, relevance_score)` sorted by score descending.
///
/// Scores are min-normalised by the maximum BM25 score so the best document
/// maps to `1.0` and a document sharing no query terms maps to `0.0`
/// (Cohere-style `[0, 1]` relevance). Ties preserve original document order.
pub fn lexical_rerank(query: &str, documents: &[String]) -> Vec<(usize, f32)> {
    const K1: f64 = 1.5;
    const B: f64 = 0.75;

    let q_terms: Vec<String> = {
        let mut t = tokenize(query);
        t.sort();
        t.dedup();
        t
    };
    let doc_tokens: Vec<Vec<String>> = documents.iter().map(|d| tokenize(d)).collect();
    let n = doc_tokens.len();
    if n == 0 {
        return Vec::new();
    }
    let avgdl = doc_tokens.iter().map(|d| d.len()).sum::<usize>() as f64 / n as f64;

    // Document frequency per query term.
    let df = |term: &str| -> usize {
        doc_tokens
            .iter()
            .filter(|d| d.iter().any(|w| w == term))
            .count()
    };

    let raw: Vec<(usize, f64)> = doc_tokens
        .iter()
        .enumerate()
        .map(|(i, tokens)| {
            let dl = tokens.len() as f64;
            let mut score = 0.0_f64;
            for term in &q_terms {
                let tf = tokens.iter().filter(|w| *w == term).count() as f64;
                if tf == 0.0 {
                    continue;
                }
                let df_t = df(term) as f64;
                // BM25 IDF (always positive via the +1 inside the log).
                let idf = (((n as f64 - df_t + 0.5) / (df_t + 0.5)) + 1.0).ln();
                let denom = tf + K1 * (1.0 - B + B * (dl / avgdl.max(1.0)));
                score += idf * (tf * (K1 + 1.0)) / denom;
            }
            (i, score)
        })
        .collect();

    // Normalise to [0, 1] by the maximum raw score.
    let max = raw.iter().fold(0.0_f64, |m, (_, s)| m.max(*s));
    let mut scored: Vec<(usize, f32)> = raw
        .into_iter()
        .map(|(i, s)| (i, if max > 0.0 { (s / max) as f32 } else { 0.0 }))
        .collect();

    // Stable sort by score descending (ties keep original index order).
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    scored
}

/// Run a rerank request through the in-process lexical scorer and build a
/// Cohere/Jina-compatible [`RerankResponse`] (applies `top_n` truncation,
/// echoes documents when `return_documents` is set, bills one search unit per
/// 100 candidate documents).
pub fn rerank_local(req: &RerankRequest) -> RerankResponse {
    let ranked = lexical_rerank(&req.query, &req.documents);
    let echo = req.return_documents.unwrap_or(false);
    let limit = req.top_n.unwrap_or(ranked.len()).min(ranked.len());

    let results = ranked
        .into_iter()
        .take(limit)
        .map(|(index, relevance_score)| RerankResult {
            index,
            relevance_score,
            document: echo.then(|| RerankDocument {
                text: req.documents[index].clone(),
            }),
        })
        .collect();

    let search_units = ((req.documents.len() + 99) / 100).max(1) as u32;

    RerankResponse {
        id: format!("rerank-{:016x}", fnv1a(&req.query)),
        results,
        model: req.model.clone(),
        meta: RerankMeta {
            billed_units: RerankBilledUnits { search_units },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_rerank_ranks_relevant_doc_first_and_scores_disjoint_zero() {
        let docs = vec![
            "Berlin is the capital of Germany.".to_string(),
            "Paris is the capital of France and a major city.".to_string(),
            "The quick brown fox jumps.".to_string(),
        ];
        let ranked = lexical_rerank("capital of france", &docs);
        // Highest-scoring document must be the France one (index 1).
        assert_eq!(ranked[0].0, 1, "France doc should rank first");
        assert!(ranked[0].1 > 0.0);
        // Scores are sorted descending.
        for w in ranked.windows(2) {
            assert!(w[0].1 >= w[1].1, "scores must be sorted descending");
        }
        // The fox doc shares no query terms -> exactly zero relevance.
        let fox = ranked.iter().find(|(i, _)| *i == 2).unwrap();
        assert_eq!(fox.1, 0.0, "disjoint document scores 0.0");
    }

    #[test]
    fn rerank_local_applies_top_n_and_returns_documents() {
        let req = RerankRequest {
            model: "rerank-english-v3.0".into(),
            query: "capital of france".into(),
            documents: vec![
                "Berlin is the capital of Germany.".into(),
                "Paris is the capital of France.".into(),
                "Unrelated text about gardening.".into(),
            ],
            top_n: Some(2),
            return_documents: Some(true),
        };
        let resp = rerank_local(&req);
        assert_eq!(resp.model, "rerank-english-v3.0");
        assert_eq!(resp.results.len(), 2, "top_n truncates to 2");
        assert_eq!(resp.results[0].index, 1, "France doc first");
        assert!(resp.results[0].document.is_some(), "documents echoed");
        assert_eq!(
            resp.results[0].document.as_ref().unwrap().text,
            "Paris is the capital of France."
        );
        // Every relevance score is a valid [0,1] fraction.
        for r in &resp.results {
            assert!((0.0..=1.0).contains(&r.relevance_score));
        }
        assert!(resp.meta.billed_units.search_units >= 1);
    }

    #[test]
    fn rerank_local_omits_documents_when_not_requested() {
        let req = RerankRequest {
            model: "rerank".into(),
            query: "france".into(),
            documents: vec!["France".into(), "Spain".into()],
            top_n: None,
            return_documents: None,
        };
        let resp = rerank_local(&req);
        assert_eq!(resp.results.len(), 2, "no top_n -> all docs");
        assert!(resp.results.iter().all(|r| r.document.is_none()));
    }

    #[test]
    fn rerank_request_deserialises_cohere_shape() {
        let req: RerankRequest = serde_json::from_str(
            r#"{"model":"rerank-english-v3.0","query":"capital of france","documents":["Paris is the capital of France.","Berlin is in Germany."],"top_n":1,"return_documents":true}"#,
        )
        .unwrap();
        assert_eq!(req.model, "rerank-english-v3.0");
        assert_eq!(req.query, "capital of france");
        assert_eq!(req.documents.len(), 2);
        assert_eq!(req.top_n, Some(1));
        assert_eq!(req.return_documents, Some(true));
    }

    #[test]
    fn rerank_response_serialises_to_cohere_list() {
        let resp = RerankResponse {
            id: "rerank-abc".into(),
            results: vec![RerankResult {
                index: 0,
                relevance_score: 0.97,
                document: Some(RerankDocument {
                    text: "Paris is the capital of France.".into(),
                }),
            }],
            model: "rerank-english-v3.0".into(),
            meta: RerankMeta {
                billed_units: RerankBilledUnits { search_units: 1 },
            },
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["results"][0]["index"], 0);
        assert!((v["results"][0]["relevance_score"].as_f64().unwrap() - 0.97).abs() < 1e-4);
        assert_eq!(
            v["results"][0]["document"]["text"],
            "Paris is the capital of France."
        );
        assert_eq!(v["meta"]["billed_units"]["search_units"], 1);
    }
}

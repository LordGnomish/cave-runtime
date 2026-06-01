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

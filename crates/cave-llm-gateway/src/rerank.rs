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

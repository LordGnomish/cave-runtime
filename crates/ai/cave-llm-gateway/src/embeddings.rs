// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenAI-compatible embeddings + rerank endpoints.
//!
//! Maps `litellm/main.py::embedding` and `litellm/rerank_api/`. The
//! `/v1/embeddings` surface is OpenAI-shaped (string-or-array `input`,
//! `data[].embedding` floats, token usage). Rerank follows the Cohere
//! `/v2/rerank` contract (query + documents -> scored indices). Request/
//! response types and the Cohere-rerank transform are ported here as pure,
//! testable units; the provider trait gains `embed` / `rerank` methods that
//! forward to the resolved backend.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_single_input_yields_one_element() {
        let n = normalize_input(&EmbeddingInput::Single("hello".into()));
        assert_eq!(n, vec!["hello".to_string()]);
    }

    #[test]
    fn normalize_multiple_input_preserves_order() {
        let n = normalize_input(&EmbeddingInput::Multiple(vec!["a".into(), "b".into()]));
        assert_eq!(n, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn embedding_request_deserializes_string_input() {
        let req: EmbeddingRequest =
            serde_json::from_str(r#"{"model":"text-embedding-3-small","input":"hi"}"#).unwrap();
        assert_eq!(normalize_input(&req.input), vec!["hi".to_string()]);
    }

    #[test]
    fn embedding_request_deserializes_array_input() {
        let req: EmbeddingRequest = serde_json::from_str(
            r#"{"model":"text-embedding-3-small","input":["x","y","z"]}"#,
        )
        .unwrap();
        assert_eq!(normalize_input(&req.input).len(), 3);
    }

    #[test]
    fn embedding_response_serializes_with_list_envelope() {
        let resp = EmbeddingResponse {
            object: "list".into(),
            data: vec![EmbeddingData {
                object: "embedding".into(),
                embedding: vec![0.1, 0.2],
                index: 0,
            }],
            model: "text-embedding-3-small".into(),
            usage: Usage::new(3, 0),
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["object"], "list");
        assert_eq!(v["data"][0]["object"], "embedding");
        assert_eq!(v["data"][0]["index"], 0);
        assert_eq!(v["usage"]["prompt_tokens"], 3);
    }

    #[test]
    fn cohere_rerank_body_includes_query_documents_and_top_n() {
        let req = RerankRequest {
            model: "rerank-v3.5".into(),
            query: "best laptop".into(),
            documents: vec!["doc a".into(), "doc b".into()],
            top_n: Some(1),
        };
        let body = to_cohere_rerank_body(&req);
        assert_eq!(body["query"], "best laptop");
        assert_eq!(body["documents"].as_array().unwrap().len(), 2);
        assert_eq!(body["top_n"], 1);
    }

    #[test]
    fn from_cohere_rerank_preserves_scores_and_order() {
        let raw = serde_json::json!({
            "results": [
                {"index": 1, "relevance_score": 0.97},
                {"index": 0, "relevance_score": 0.12}
            ]
        });
        let out = from_cohere_rerank(&raw, "rerank-v3.5");
        assert_eq!(out.results.len(), 2);
        assert_eq!(out.results[0].index, 1);
        assert!((out.results[0].relevance_score - 0.97).abs() < 1e-6);
        assert_eq!(out.results[1].index, 0);
        assert_eq!(out.model, "rerank-v3.5");
    }
}

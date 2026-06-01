// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rerank endpoint (`POST /v1/rerank`) — Cohere/Jina-compatible cross-encoder
//! relevance scoring.

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
        assert_eq!(v["results"][0]["relevance_score"].as_f64().unwrap(), 0.97);
        assert_eq!(
            v["results"][0]["document"]["text"],
            "Paris is the capital of France."
        );
        assert_eq!(v["meta"]["billed_units"]["search_units"], 1);
    }
}

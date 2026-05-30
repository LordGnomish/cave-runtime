// SPDX-License-Identifier: AGPL-3.0-or-later
//! Re-rankers: lexical cross-encoder + LLM-as-judge.

use cave_rag::document::Document;
use cave_rag::error::Result;
use cave_rag::rerank::{LexicalCrossEncoder, LlmClient, LlmJudgeReranker, Reranker};
use cave_rag::vectorstore::ScoredDocument;

fn candidates() -> Vec<ScoredDocument> {
    vec![
        ScoredDocument {
            document: Document::new("python is a dynamic language with duck typing"),
            score: 0.95, // misleadingly high prior
        },
        ScoredDocument {
            document: Document::new("rust guarantees memory safety without a garbage collector"),
            score: 0.10, // misleadingly low prior
        },
    ]
}

#[test]
fn cross_encoder_promotes_high_query_coverage_doc() {
    let r = LexicalCrossEncoder::new();
    let out = r
        .rerank("rust memory safety", candidates(), 2)
        .unwrap();
    assert_eq!(out.len(), 2);
    assert!(
        out[0].document.content.contains("memory safety"),
        "doc covering all query terms must win despite a lower prior"
    );
    assert!(out[0].score >= out[1].score);
}

#[test]
fn cross_encoder_can_truncate_to_top_n() {
    let r = LexicalCrossEncoder::new();
    let out = r.rerank("rust memory safety", candidates(), 1).unwrap();
    assert_eq!(out.len(), 1);
}

/// A deterministic LLM stand-in: returns the score whose trigger substring
/// appears in the prompt (the prompt embeds the document text).
struct ScriptedLlm {
    rules: Vec<(&'static str, f64)>,
}

impl LlmClient for ScriptedLlm {
    fn complete(&self, prompt: &str) -> Result<String> {
        for (needle, score) in &self.rules {
            if prompt.contains(needle) {
                return Ok(format!("Relevance score: {score}"));
            }
        }
        Ok("0".to_string())
    }
}

#[test]
fn llm_judge_reranks_by_parsed_score() {
    let llm = ScriptedLlm {
        rules: vec![("memory safety", 9.0), ("dynamic language", 2.0)],
    };
    let r = LlmJudgeReranker::new(&llm);
    let out = r
        .rerank("which language is memory safe", candidates(), 2)
        .unwrap();
    assert_eq!(out.len(), 2);
    assert!(
        out[0].document.content.contains("memory safety"),
        "LLM judge should rank the higher-scored doc first"
    );
    assert!(out[0].score > out[1].score);
}

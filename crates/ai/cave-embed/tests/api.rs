// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 5 — OpenAI /v1/embeddings API + EmbeddingService.
//
// The service ties registry + backend + tokenizer together: resolve the model
// card, truncate each input to the context window, embed, optionally truncate
// dimensions (Matryoshka) and renormalize, encode float or base64, and report
// token usage — mirroring the OpenAI embeddings contract infinity serves.

use cave_embed::api::{EmbeddingData, EmbeddingRequest, Input};
use cave_embed::service::{EmbeddingService, ServiceError};

fn svc() -> EmbeddingService {
    EmbeddingService::with_builtins()
}

#[test]
fn input_deserializes_string_or_array() {
    let one: EmbeddingRequest =
        serde_json::from_str(r#"{"model":"minilm","input":"hello"}"#).unwrap();
    assert_eq!(one.input.into_vec(), vec!["hello".to_string()]);
    let many: EmbeddingRequest =
        serde_json::from_str(r#"{"model":"minilm","input":["a","b"]}"#).unwrap();
    assert_eq!(many.input.into_vec(), vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn single_input_returns_one_embedding() {
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Single("hello world".into()),
        encoding_format: None,
        dimensions: None,
        user: None,
    };
    let resp = svc().embed(&req).unwrap();
    assert_eq!(resp.data.len(), 1);
    assert_eq!(resp.data[0].index, 0);
    assert_eq!(resp.object, "list");
    // canonical model id echoed back
    assert_eq!(resp.model, "sentence-transformers/all-MiniLM-L6-v2");
    match &resp.data[0].embedding {
        EmbeddingData::Float(v) => assert_eq!(v.len(), 384),
        _ => panic!("expected float embedding"),
    }
    assert!(resp.usage.prompt_tokens >= 2);
    assert_eq!(resp.usage.total_tokens, resp.usage.prompt_tokens);
}

#[test]
fn array_input_indexes_in_order() {
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Batch(vec!["one".into(), "two".into(), "three".into()]),
        encoding_format: None,
        dimensions: None,
        user: None,
    };
    let resp = svc().embed(&req).unwrap();
    assert_eq!(resp.data.len(), 3);
    let idx: Vec<usize> = resp.data.iter().map(|d| d.index).collect();
    assert_eq!(idx, vec![0, 1, 2]);
}

#[test]
fn unknown_model_errors() {
    let req = EmbeddingRequest {
        model: "does-not-exist".into(),
        input: Input::Single("x".into()),
        encoding_format: None,
        dimensions: None,
        user: None,
    };
    assert!(matches!(svc().embed(&req), Err(ServiceError::UnknownModel(_))));
}

#[test]
fn base64_encoding_round_trips_to_dims() {
    use base64::Engine;
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Single("hello".into()),
        encoding_format: Some("base64".into()),
        dimensions: None,
        user: None,
    };
    let resp = svc().embed(&req).unwrap();
    match &resp.data[0].embedding {
        EmbeddingData::Base64(s) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(s).unwrap();
            assert_eq!(bytes.len(), 384 * 4, "f32 little-endian payload");
        }
        _ => panic!("expected base64 embedding"),
    }
}

#[test]
fn dimensions_truncates_and_renormalizes() {
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Single("alpha beta gamma".into()),
        encoding_format: None,
        dimensions: Some(64),
        user: None,
    };
    let resp = svc().embed(&req).unwrap();
    match &resp.data[0].embedding {
        EmbeddingData::Float(v) => {
            assert_eq!(v.len(), 64, "Matryoshka truncation to requested dims");
            let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((mag - 1.0).abs() < 1e-5, "renormalized after truncation");
        }
        _ => panic!("expected float embedding"),
    }
}

#[test]
fn requesting_more_dims_than_model_errors() {
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Single("x".into()),
        encoding_format: None,
        dimensions: Some(99999),
        user: None,
    };
    assert!(matches!(
        svc().embed(&req),
        Err(ServiceError::InvalidDimensions { .. })
    ));
}

#[test]
fn long_input_is_truncated_not_rejected() {
    let long = "word ".repeat(5000);
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Single(long),
        encoding_format: None,
        dimensions: None,
        user: None,
    };
    let resp = svc().embed(&req).unwrap();
    // MiniLM context window is 256 tokens — usage must be capped there.
    assert!(resp.usage.prompt_tokens <= 256);
    assert_eq!(resp.data.len(), 1);
}

#[test]
fn empty_input_list_errors() {
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Batch(vec![]),
        encoding_format: None,
        dimensions: None,
        user: None,
    };
    assert!(matches!(svc().embed(&req), Err(ServiceError::EmptyInput)));
}

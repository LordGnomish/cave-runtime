// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration test for `OpenAiCompatClient::chat_completions_stream` against an
//! in-process axum mock that emits OpenAI SSE `chat.completion.chunk` frames.
//! Cite docs/openai.md — `/v1/chat/completions` with `stream: true`.

use axum::{Router, routing::post};
use cave_local_llm::openai_compat::{OpenAiChatMessage, OpenAiChatRequest, OpenAiCompatClient};
use futures::StreamExt;
use tokio::net::TcpListener;

async fn spawn_mock_server(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn test_chat_completions_stream_collects_deltas() {
    // Two content deltas + a finish frame + the [DONE] terminator.
    let sse = concat!(
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || async move {
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                sse,
            )
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let req = OpenAiChatRequest {
        model: "m".into(),
        messages: vec![OpenAiChatMessage {
            role: "user".into(),
            content: "hi".into(),
        }],
        temperature: None,
        max_tokens: None,
        top_p: None,
        stream: Some(true),
        seed: None,
        stop: None,
    };

    let mut stream = OpenAiCompatClient::new(base_url)
        .chat_completions_stream(req)
        .await
        .expect("stream should open");

    let mut content = String::new();
    let mut finish = None;
    while let Some(item) = stream.next().await {
        let chunk = item.expect("chunk ok");
        if let Some(c) = chunk.choices.first() {
            if let Some(delta) = c.delta.content.as_deref() {
                content.push_str(delta);
            }
            if c.finish_reason.is_some() {
                finish = c.finish_reason.clone();
            }
        }
    }

    assert_eq!(content, "Hello");
    assert_eq!(finish.as_deref(), Some("stop"));
}

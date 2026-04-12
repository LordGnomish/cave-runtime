//! SSE streaming support for chat completions.

use crate::openai::ChatCompletionChunk;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{self, Stream, StreamExt};
use serde::Serialize;
use std::convert::Infallible;
use std::time::Duration;

/// Format a chunk as an SSE data event.
pub fn chunk_to_sse_event(chunk: &ChatCompletionChunk) -> Result<Event, Infallible> {
    let data = serde_json::to_string(chunk).unwrap_or_else(|_| "{}".into());
    Ok(Event::default().data(data))
}

/// The terminal SSE event: `data: [DONE]`
pub fn done_event() -> Result<Event, Infallible> {
    Ok(Event::default().data("[DONE]"))
}

/// Build an SSE stream from a vec of chunks (used for non-streaming providers
/// that produce a full response which we then re-stream chunk by chunk).
pub fn chunks_to_sse_stream(
    chunks: Vec<ChatCompletionChunk>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let events = chunks
        .into_iter()
        .map(|c| chunk_to_sse_event(&c))
        .chain(std::iter::once(done_event()));

    Sse::new(stream::iter(events)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Split a full completion text into streaming chunks for simulated streaming.
pub fn simulate_streaming(id: &str, model: &str, text: &str) -> Vec<ChatCompletionChunk> {
    // Split on word boundaries for a natural feel
    let words: Vec<&str> = text.split_inclusive(' ').collect();
    let mut chunks: Vec<ChatCompletionChunk> = words
        .iter()
        .map(|word| ChatCompletionChunk::content_delta(id, model, word))
        .collect();
    chunks.push(ChatCompletionChunk::stop(id, model));
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulate_streaming_produces_stop_chunk() {
        let chunks = simulate_streaming("id-123", "gpt-4o", "Hello world");
        let last = chunks.last().unwrap();
        assert!(last.choices[0].finish_reason.as_deref() == Some("stop"));
    }

    #[test]
    fn simulate_streaming_content_chunks() {
        let chunks = simulate_streaming("id-1", "gpt-4o", "Hello world");
        // Content chunks (all but last)
        let content: String = chunks[..chunks.len() - 1]
            .iter()
            .filter_map(|c| c.choices[0].delta.as_ref())
            .filter_map(|d| d.content.as_deref())
            .collect();
        assert_eq!(content, "Hello world");
    }

    #[test]
    fn chunk_to_sse_is_valid_json() {
        let chunk = ChatCompletionChunk::content_delta("id-1", "gpt-4o", "hi");
        let event = chunk_to_sse_event(&chunk).unwrap();
        // If we can serialize the chunk, the event should be fine
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("chat.completion.chunk"));
    }
}

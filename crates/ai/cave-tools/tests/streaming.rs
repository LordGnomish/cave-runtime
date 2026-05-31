// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 7 (RED→GREEN): streaming tool results.

use cave_tools::streaming::{collect_stream, FnStreamingTool, StreamChunk, StreamingTool};
use cave_tools::tool::{Tool, ToolRegistry, ToolResult};
use serde_json::json;

fn counter() -> FnStreamingTool {
    FnStreamingTool::new(
        "count",
        "emit n chunks then a final tally",
        json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]}),
        |args, sink| {
            let n = args["n"].as_u64().unwrap_or(0);
            for i in 0..n {
                sink(StreamChunk::new(i, format!("tick {i}")));
            }
            Ok(ToolResult::text(format!("done: {n}")))
        },
    )
}

#[test]
fn chunks_are_emitted_in_order() {
    let t = counter();
    let mut seen = Vec::new();
    let final_res = t
        .stream(&json!({"n": 3}), &mut |c: StreamChunk| seen.push(c))
        .unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].seq, 0);
    assert_eq!(seen[2].seq, 2);
    assert_eq!(seen[1].text, "tick 1");
    assert_eq!(final_res.text_output(), "done: 3");
}

#[test]
fn collect_stream_returns_chunks_and_final() {
    let t = counter();
    let (chunks, final_res) = collect_stream(&t, &json!({"n": 2})).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(final_res.text_output(), "done: 2");
}

#[test]
fn chunk_maps_to_mcp_progress_notification() {
    let c = StreamChunk::new(4, "halfway");
    let note = c.progress_notification("tok-123", Some(10));
    assert_eq!(note["jsonrpc"], "2.0");
    assert_eq!(note["method"], "notifications/progress");
    assert_eq!(note["params"]["progressToken"], "tok-123");
    // progress is 1-based count of chunks emitted (seq + 1).
    assert_eq!(note["params"]["progress"], 5);
    assert_eq!(note["params"]["total"], 10);
    assert_eq!(note["params"]["message"], "halfway");
}

#[test]
fn buffered_adapter_makes_streaming_tool_a_regular_tool() {
    // A streaming tool can be exposed through the normal Tool interface,
    // concatenating its chunks ahead of the final result.
    let t = counter();
    let buffered = t.into_buffered();
    assert_eq!(buffered.name(), "count");
    let mut reg = ToolRegistry::new();
    reg.register(buffered);
    let out = reg.invoke_validated("count", &json!({"n": 2})).unwrap();
    // Chunks + final, newline-joined.
    assert_eq!(out.text_output(), "tick 0\ntick 1\ndone: 2");
}

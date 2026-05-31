// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Streaming (incremental) tool results.
//!
//! A [`StreamingTool`] emits an ordered series of [`StreamChunk`]s through a
//! caller-supplied sink and then returns a final [`ToolResult`]. This maps
//! onto MCP's progress notifications (`notifications/progress`) — each chunk
//! can be rendered as one via [`StreamChunk::progress_notification`] — while
//! the final result is the eventual `tools/call` response.
//!
//! The sink is a `&mut dyn FnMut(StreamChunk)`, so the framework stays
//! runtime-agnostic: a synchronous caller collects into a `Vec`, an async
//! transport forwards each chunk over the wire as it arrives.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::Result;
use crate::tool::{FnTool, ToolResult};

/// One incremental piece of a streaming result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamChunk {
    /// 0-based ordinal of this chunk within the stream.
    pub seq: u64,
    /// Text payload of the chunk.
    pub text: String,
}

impl StreamChunk {
    pub fn new(seq: u64, text: impl Into<String>) -> Self {
        Self {
            seq,
            text: text.into(),
        }
    }

    /// Render this chunk as an MCP `notifications/progress` message bound to
    /// `token`. `progress` is the 1-based count of chunks emitted so far
    /// (`seq + 1`); `total`, when known, lets clients show a progress bar.
    pub fn progress_notification(&self, token: &str, total: Option<u64>) -> Value {
        let mut params = json!({
            "progressToken": token,
            "progress": self.seq + 1,
            "message": self.text,
        });
        if let Some(t) = total {
            params["total"] = json!(t);
        }
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": params
        })
    }
}

/// A tool that produces incremental output.
pub trait StreamingTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn toolset(&self) -> &str {
        "general"
    }

    /// Run the tool, emitting chunks through `sink`, and return the final
    /// aggregate result.
    fn stream(&self, args: &Value, sink: &mut dyn FnMut(StreamChunk)) -> Result<ToolResult>;
}

/// A [`StreamingTool`] built from a closure.
pub struct FnStreamingTool {
    name: String,
    description: String,
    schema: Value,
    toolset: String,
    #[allow(clippy::type_complexity)]
    handler: Box<dyn Fn(&Value, &mut dyn FnMut(StreamChunk)) -> Result<ToolResult> + Send + Sync>,
}

impl FnStreamingTool {
    pub fn new<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: Value,
        handler: F,
    ) -> Self
    where
        F: Fn(&Value, &mut dyn FnMut(StreamChunk)) -> Result<ToolResult> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
            toolset: "general".to_string(),
            handler: Box::new(handler),
        }
    }

    pub fn with_toolset(mut self, toolset: impl Into<String>) -> Self {
        self.toolset = toolset.into();
        self
    }

    /// Wrap this streaming tool as a regular buffering [`FnTool`]: it runs
    /// the stream, concatenating every chunk's text ahead of the final
    /// result's text. Lets a streaming tool live in a normal
    /// [`ToolRegistry`](crate::tool::ToolRegistry).
    pub fn into_buffered(self) -> FnTool {
        let name = self.name.clone();
        let description = self.description.clone();
        let schema = self.schema.clone();
        let toolset = self.toolset.clone();
        FnTool::new(name, description, schema, move |args| {
            let mut chunks: Vec<StreamChunk> = Vec::new();
            let final_res = self.stream(args, &mut |c| chunks.push(c))?;
            let mut lines: Vec<String> = chunks.into_iter().map(|c| c.text).collect();
            lines.push(final_res.text_output());
            Ok(ToolResult {
                content: vec![crate::tool::Content::text(lines.join("\n"))],
                is_error: final_res.is_error,
                structured: final_res.structured,
            })
        })
        .with_toolset(toolset)
    }
}

impl StreamingTool for FnStreamingTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> Value {
        self.schema.clone()
    }
    fn toolset(&self) -> &str {
        &self.toolset
    }
    fn stream(&self, args: &Value, sink: &mut dyn FnMut(StreamChunk)) -> Result<ToolResult> {
        (self.handler)(args, sink)
    }
}

/// Drive a [`StreamingTool`] to completion, collecting every chunk and the
/// final result.
pub fn collect_stream(
    tool: &dyn StreamingTool,
    args: &Value,
) -> Result<(Vec<StreamChunk>, ToolResult)> {
    let mut chunks = Vec::new();
    let final_res = tool.stream(args, &mut |c| chunks.push(c))?;
    Ok((chunks, final_res))
}

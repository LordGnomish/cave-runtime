// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The core tool abstraction: the [`Tool`] trait, its serializable
//! descriptor [`ToolSpec`], the MCP-shaped [`ToolResult`], and the
//! [`ToolRegistry`] that owns a set of tools and dispatches calls.
//!
//! Mirrors the MCP `Tool` object (`schema/2025-11-25/schema.ts`):
//! `{ name, title?, description?, inputSchema, annotations? }`, and the
//! `CallToolResult` `{ content[], isError, structuredContent? }`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, ToolError};

/// The contents an [`Content::Resource`] (MCP `EmbeddedResource`) carries.
/// Mirrors the `TextResourceContents | BlobResourceContents` union from
/// `schema/2025-11-25/schema.ts`: both share `uri` + optional `mimeType`,
/// and carry *either* inline `text` or base64 `blob` (never both).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceContents {
    pub uri: String,
    #[serde(
        rename = "mimeType",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
    /// Base64-encoded binary payload.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub blob: Option<String>,
}

/// A single content block in a tool result, modelling the full MCP
/// `ContentBlock` union from `schema/2025-11-25/schema.ts`:
/// `TextContent | ImageContent | AudioContent | ResourceLink |
/// EmbeddedResource`. Image/audio carry base64 `data` + `mimeType`; the
/// embedded `Resource` carries inline text or a base64 blob; `ResourceLink`
/// references a resource the client can fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    Image {
        /// Base64-encoded image bytes.
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    Audio {
        /// Base64-encoded audio bytes.
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    /// MCP `EmbeddedResource` — resource data inlined into the result.
    Resource {
        resource: ResourceContents,
    },
    /// MCP `ResourceLink` — a reference the client can resolve via
    /// `resources/read`.
    ResourceLink {
        uri: String,
        name: String,
    },
}

impl Content {
    pub fn text(s: impl Into<String>) -> Self {
        Content::Text { text: s.into() }
    }

    /// An image block from base64 `data` and its MIME type.
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Content::Image {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    /// An audio block from base64 `data` and its MIME type.
    pub fn audio(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Content::Audio {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    /// An embedded resource carrying inline `text`.
    pub fn resource_text(
        uri: impl Into<String>,
        mime_type: Option<&str>,
        text: impl Into<String>,
    ) -> Self {
        Content::Resource {
            resource: ResourceContents {
                uri: uri.into(),
                mime_type: mime_type.map(str::to_string),
                text: Some(text.into()),
                blob: None,
            },
        }
    }

    /// An embedded resource carrying a base64 `blob`.
    pub fn resource_blob(
        uri: impl Into<String>,
        mime_type: Option<&str>,
        blob: impl Into<String>,
    ) -> Self {
        Content::Resource {
            resource: ResourceContents {
                uri: uri.into(),
                mime_type: mime_type.map(str::to_string),
                text: None,
                blob: Some(blob.into()),
            },
        }
    }

    /// A link to a resource the client can fetch.
    pub fn resource_link(uri: impl Into<String>, name: impl Into<String>) -> Self {
        Content::ResourceLink {
            uri: uri.into(),
            name: name.into(),
        }
    }

    /// The textual projection of this block, if any. Only [`Content::Text`]
    /// contributes prose; binary/resource blocks return `None`.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }
}

/// Result of a tool invocation, shaped exactly like MCP's `CallToolResult`
/// so the [`crate::mcp`] server can return it verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    /// Ordered content blocks returned to the model.
    pub content: Vec<Content>,
    /// `true` when the tool reported a domain error (still a successful
    /// JSON-RPC response — the error is surfaced *to the model*, per MCP).
    #[serde(rename = "isError", default)]
    pub is_error: bool,
    /// Optional machine-readable result payload (MCP `structuredContent`).
    #[serde(
        rename = "structuredContent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub structured: Option<Value>,
}

impl ToolResult {
    /// A successful single-text result.
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(s)],
            is_error: false,
            structured: None,
        }
    }

    /// An error result surfaced to the model (`isError = true`).
    pub fn error(s: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(s)],
            is_error: true,
            structured: None,
        }
    }

    /// Attach a machine-readable payload.
    pub fn with_structured(mut self, v: Value) -> Self {
        self.structured = Some(v);
        self
    }

    /// Concatenate all text blocks (newline-joined) — convenience for
    /// callers that just want the textual answer.
    pub fn text_output(&self) -> String {
        self.content
            .iter()
            .filter_map(Content::as_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The callable contract every tool implements.
///
/// `input_schema` is a JSON Schema object describing the `arguments` the
/// model must supply; [`crate::schema`] validates calls against it before
/// `execute` runs (see [`ToolRegistry::invoke`]).
pub trait Tool: Send + Sync {
    /// Unique identifier exposed to the model.
    fn name(&self) -> &str;

    /// Human-readable description (used by the model to choose the tool).
    fn description(&self) -> &str;

    /// JSON Schema for the `arguments` object.
    fn input_schema(&self) -> Value;

    /// Run the tool against validated `args`.
    fn execute(&self, args: &Value) -> Result<ToolResult>;

    /// Coarse grouping used for permissions and search facets. Defaults
    /// to `"general"`.
    fn toolset(&self) -> &str {
        "general"
    }
}

/// Serializable descriptor of a tool, matching the MCP `tools/list` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// Cave extension (non-MCP): the tool's coarse grouping.
    #[serde(default)]
    pub toolset: String,
}

impl ToolSpec {
    pub fn of(tool: &dyn Tool) -> Self {
        Self {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            input_schema: tool.input_schema(),
            toolset: tool.toolset().to_string(),
        }
    }
}

/// Type of a closure-backed tool handler.
pub type ToolHandler = Arc<dyn Fn(&Value) -> Result<ToolResult> + Send + Sync>;

/// A [`Tool`] built from a plain closure — the workhorse adapter used by
/// built-ins and tests. Mirrors LangChain's `StructuredTool.from_function`.
pub struct FnTool {
    name: String,
    description: String,
    schema: Value,
    toolset: String,
    handler: ToolHandler,
}

impl FnTool {
    pub fn new<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: Value,
        handler: F,
    ) -> Self
    where
        F: Fn(&Value) -> Result<ToolResult> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
            toolset: "general".to_string(),
            handler: Arc::new(handler),
        }
    }

    pub fn with_toolset(mut self, toolset: impl Into<String>) -> Self {
        self.toolset = toolset.into();
        self
    }
}

impl Tool for FnTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> Value {
        self.schema.clone()
    }
    fn execute(&self, args: &Value) -> Result<ToolResult> {
        (self.handler)(args)
    }
    fn toolset(&self) -> &str {
        &self.toolset
    }
}

/// Owns a named set of tools and dispatches calls. Iteration order is
/// stable (BTreeMap) so `tools/list` output and tests are deterministic.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    inner: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or replace) a tool, returning the previous entry if any.
    pub fn register(&mut self, tool: impl Tool + 'static) -> Option<Arc<dyn Tool>> {
        let name = tool.name().to_string();
        self.inner.insert(name, Arc::new(tool))
    }

    /// Insert an already-`Arc`'d tool (used when sharing instances).
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> Option<Arc<dyn Tool>> {
        self.inner.insert(tool.name().to_string(), tool)
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.inner.get(name)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.keys().cloned().collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.inner.values()
    }

    /// Descriptors for every tool, in name order — the body of an MCP
    /// `tools/list` response.
    pub fn list_specs(&self) -> Vec<ToolSpec> {
        self.inner.values().map(|t| ToolSpec::of(t.as_ref())).collect()
    }

    /// Dispatch a call. Resolves the tool then runs its handler. Argument
    /// validation against the tool's JSON Schema is layered on by
    /// [`crate::schema`] (see `invoke_validated`); this raw path is used
    /// when the caller has already validated.
    pub fn invoke(&self, name: &str, args: &Value) -> Result<ToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::NotFound(name.to_string()))?;
        tool.execute(args)
    }

    /// Dispatch a call after validating `args` against the tool's JSON
    /// Schema. Invalid arguments are rejected with
    /// [`ToolError::InvalidArguments`] *before* the handler runs. This is
    /// the path the MCP server and batch executor use.
    pub fn invoke_validated(&self, name: &str, args: &Value) -> Result<ToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::NotFound(name.to_string()))?;
        crate::schema::validate_args(name, &tool.input_schema(), args)?;
        tool.execute(args)
    }
}

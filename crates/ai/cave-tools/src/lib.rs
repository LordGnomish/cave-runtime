// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! # cave-tools
//!
//! An LLM tool-calling framework for the Cave runtime, built to be
//! drop-in compatible with the [Model Context Protocol] (MCP) and the
//! OpenAI function-calling / LangChain tool conventions.
//!
//! The crate is organised as a stack of independent layers:
//!
//! * [`tool`] — the [`Tool`](tool::Tool) trait (`name` + `schema` +
//!   `execute`), [`ToolSpec`](tool::ToolSpec), and the [`ToolRegistry`].
//! * [`schema`] — a self-contained JSON Schema validator used to gate
//!   tool arguments before a handler ever runs.
//! * [`mcp`] — a JSON-RPC 2.0 server exposing the registry over the MCP
//!   `initialize` / `tools/list` / `tools/call` methods.
//! * [`permission`] — per-tool and per-user authorization.
//! * [`audit`] — an append-only invocation history / audit log.
//! * [`builtin`] — sandboxed built-in tools (file ops, web search, code
//!   execution, calendar, email).
//! * [`streaming`] — incremental (chunked) tool results.
//! * [`batch`] — fan-out execution of many calls in one request.
//! * [`search`] — semantic tool discovery over the registry.
//! * [`adapters`] — OpenAI function-calling and LangChain interop.
//!
//! [Model Context Protocol]: https://modelcontextprotocol.io

pub mod error;
pub mod schema;
pub mod tool;

pub use error::{Result, ToolError};
pub use tool::{Content, FnTool, Tool, ToolRegistry, ToolResult, ToolSpec};

/// Pinned MCP specification revision this crate targets.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

/// Crate version surfaced over the wire (server `serverInfo.version`).
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

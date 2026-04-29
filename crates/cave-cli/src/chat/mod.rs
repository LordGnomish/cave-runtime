//! Chat module — REPL state machine + conversation resolution + tool mode + pipe output.
//!
//! ratatui is used by the binary for rendering; the state machine here is
//! transport- and renderer-agnostic so tests can drive it with synthetic
//! `ReplCommand` values without spinning up a terminal.

pub mod conversation;
pub mod output;
pub mod repl;
pub mod tool_mode;

pub use conversation::{
    Conversation, ConversationKind, ConversationStore, InMemoryConversationStore,
};
pub use output::{ChatMessage, ChatRole, StreamChunk};
pub use repl::{ReplCommand, ReplEffect, ReplState};
pub use tool_mode::{ToolCall, ToolMode, ToolResult};

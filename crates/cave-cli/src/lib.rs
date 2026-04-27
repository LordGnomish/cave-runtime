//! cavectl library — testable surface for chat REPL, tenant/env lifecycle,
//! approval workflow, audit query, watch/stream output, and the native +
//! compatibility command surfaces (per ADR-RUNTIME-CLI-CONSOLIDATION-001).
//!
//! The `cavectl` binary uses these modules; testers consume them directly with
//! the in-memory backend implementations to avoid wire-level mocks.

pub mod approval;
pub mod audit;
pub mod chat;
pub mod compat;
pub mod env;
pub mod native;
pub mod tenant;
pub mod tui;
pub mod watch;

pub use approval::{ApprovalBackend, ApprovalRecord, ApprovalState, InMemoryApprovals};
pub use audit::{AuditEntry, AuditFilter, AuditQuery, InMemoryAuditLog};
pub use chat::{
    ChatMessage, ChatRole, Conversation, ConversationKind, ConversationStore,
    InMemoryConversationStore, ReplCommand, ReplEffect, ReplState, StreamChunk, ToolCall,
    ToolMode, ToolResult,
};
pub use env::{EnvBackend, EnvLifecycleState, EnvRecord, InMemoryEnvBackend};
pub use tenant::{
    InMemoryTenantBackend, LifecycleEvent, TenantBackend, TenantLifecycleState, TenantRecord,
};
pub use watch::{ExitCode, JsonStream, StreamFormat, WatchEvent, WatchTicker};

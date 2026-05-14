// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cavectl library — testable surface for chat REPL, tenant/env lifecycle,
//! approval workflow, audit query, watch/stream output, and the native +
//! compatibility command surfaces (per ADR-RUNTIME-CLI-CONSOLIDATION-001).
//!
//! The `cavectl` binary uses these modules; testers consume them directly with
//! the in-memory backend implementations to avoid wire-level mocks.

/// The approval workflow module, providing backend traits and in-memory
/// implementations for managing approval records and states.
pub mod approval;

/// The audit logging module, providing structures for audit entries, filters,
/// queries, and an in-memory audit log backend.
pub mod audit;

/// The chat REPL module, providing types for conversations, messages, tool
/// calls, and the REPL state machine and effects.
pub mod chat;

/// The compatibility command surface module, providing legacy command
/// implementations for backward compatibility.
pub mod compat;

/// The environment lifecycle module, providing backend traits and in-memory
/// implementations for managing environment records and states.
pub mod env;

/// The native command surface module, providing modern command implementations
/// for the cave runtime CLI.
pub mod native;

/// The shell module, providing utilities for shell integration and execution.
pub mod shell;

/// The telemetry module, providing utilities for collecting and reporting
/// telemetry data from the CLI.
pub mod telemetry;

/// The tenant lifecycle module, providing backend traits and in-memory
/// implementations for managing tenant records and states.
pub mod tenant;

/// The tenant scope module, providing types and utilities for managing
/// tenant-specific scopes and permissions.
pub mod tenant_scope;

/// The TUI (Text User Interface) module, providing components for interactive
/// terminal-based user interfaces.
pub mod tui;

/// The watch/stream module, providing types for watching changes and
/// streaming output in various formats.
pub mod watch;

/// Re-exports the approval backend traits and in-memory implementation.
pub use approval::{ApprovalBackend, ApprovalRecord, ApprovalState, InMemoryApprovals};

/// Re-exports the audit log types, filters, queries, and in-memory backend.
pub use audit::{AuditEntry, AuditFilter, AuditQuery, InMemoryAuditLog};

/// Re-exports chat-related types including messages, roles, conversations,
/// store implementations, and REPL components.
pub use chat::{
    ChatMessage, ChatRole, Conversation, ConversationKind, ConversationStore,
    InMemoryConversationStore, ReplCommand, ReplEffect, ReplState, StreamChunk, ToolCall,
    ToolMode, ToolResult,
};

/// Re-exports the environment backend traits and in-memory implementation.
pub use env::{EnvBackend, EnvLifecycleState, EnvRecord, InMemoryEnvBackend};

/// Re-exports the tenant backend traits and in-memory implementation.
pub use tenant::{
    InMemoryTenantBackend, LifecycleEvent, TenantBackend, TenantLifecycleState, TenantRecord,
};

/// Re-exports watch and stream related types including exit codes, JSON
/// streaming, formats, events, and tickers.
pub use watch::{ExitCode, JsonStream, StreamFormat, WatchEvent, WatchTicker};

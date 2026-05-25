// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
pub mod memory;
pub mod tcp;

use crate::error::HaResult;
use crate::raft::messages::RaftMessage;
use crate::raft::types::NodeId;
use async_trait::async_trait;

/// Transport abstraction — sends Raft messages between nodes.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Send a message to the given node. Fire-and-forget; errors are logged but not fatal.
    async fn send(&self, to: NodeId, msg: RaftMessage) -> HaResult<()>;
}

pub use memory::MemTransport;
pub use tcp::TcpTransport;

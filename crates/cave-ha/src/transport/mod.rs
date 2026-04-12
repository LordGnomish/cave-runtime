pub mod memory;
pub mod tcp;

use async_trait::async_trait;
use crate::error::HaResult;
use crate::raft::messages::RaftMessage;
use crate::raft::types::NodeId;

/// Transport abstraction — sends Raft messages between nodes.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Send a message to the given node. Fire-and-forget; errors are logged but not fatal.
    async fn send(&self, to: NodeId, msg: RaftMessage) -> HaResult<()>;
}

pub use memory::MemTransport;
pub use tcp::TcpTransport;

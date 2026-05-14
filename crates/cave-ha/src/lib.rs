// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE HA/DR — production-grade Raft consensus, HA failover, and cross-region DR.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         cave-ha                                      │
//! │                                                                       │
//! │  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐             │
//! │  │  Raft Engine │   │  HA Manager  │   │  DR Manager  │             │
//! │  │              │   │              │   │              │             │
//! │  │ • Election   │   │ • Failover   │   │ • X-region   │             │
//! │  │ • Replication│   │ • Health     │   │   replication│             │
//! │  │ • Compaction │   │ • Split-brain│   │ • PITR       │             │
//! │  │ • ReadIndex  │   │   prevention │   │ • Failback   │             │
//! │  └──────┬───────┘   └──────────────┘   └──────────────┘             │
//! │         │                                                             │
//! │  ┌──────▼───────┐   ┌──────────────┐   ┌──────────────┐             │
//! │  │   Storage    │   │  Transport   │   │   HTTP API   │             │
//! │  │ • WAL        │   │ • TCP (prod) │   │ • /status    │             │
//! │  │ • Snapshots  │   │ • Memory     │   │ • /metrics   │             │
//! │  │ • Log store  │   │   (tests)    │   │ • /members   │             │
//! │  └──────────────┘   └──────────────┘   └──────────────┘             │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```

pub mod api;
pub mod config;
pub mod dr;
pub mod error;
pub mod ha;
pub mod metrics;
pub mod raft;
pub mod storage;
pub mod transport;

pub use config::{DrConfig, NodeConfig};
pub use error::{HaError, HaResult};
pub use metrics::Metrics;
pub use raft::node::{NodeCmd, RaftHandle};
pub use raft::state_machine::{KvStateMachine, NoopStateMachine, StateMachine};
pub use raft::types::{
    EntryType, HardState, LogIndex, MembershipConfig, NodeId, NodeInfo, NodeStatus,
    Role, SnapshotMeta, Term,
};
pub use transport::memory::{MemNetwork, MemTransport};
pub use transport::Transport;

pub const MODULE_NAME: &str = "ha";

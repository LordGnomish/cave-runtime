// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! State machine surface — adopted onto `cave_kernel::consensus` (sweep-002 F2-A).
//!
//! The local `StateMachine` trait that previously lived here was a structural
//! duplicate of `cave_kernel::consensus::StateMachine`. It has been removed
//! and the kernel trait is re-exported as the canonical surface. Concrete
//! implementations in this module (`NoopStateMachine`, `KvStateMachine`) now
//! implement the kernel trait directly — apply receives a
//! `cave_kernel::consensus::LogEntry`, errors are mapped to
//! `ConsensusError::Storage`, and the cave-ha `LogEntry` (with `entry_type`)
//! is converted to the kernel projection by the node loop before invocation.

use async_trait::async_trait;
use cave_kernel::consensus::{ConsensusError, ConsensusResult, LogEntry};

/// State machine trait re-exported from `cave_kernel::consensus`. This was
/// previously a duplicate trait local to cave-ha; sweep-002 F2-A removed
/// the duplicate and aligned downstream code on the kernel surface.
pub use cave_kernel::consensus::StateMachine;

/// A no-op state machine for testing and bootstrapping.
#[derive(Default)]
pub struct NoopStateMachine;

#[async_trait]
impl StateMachine for NoopStateMachine {
    async fn apply(&self, _entry: &LogEntry) -> ConsensusResult<Vec<u8>> {
        Ok(vec![])
    }

    async fn snapshot(&self) -> ConsensusResult<Vec<u8>> {
        Ok(vec![])
    }

    async fn restore(&self, _data: &[u8]) -> ConsensusResult<()> {
        Ok(())
    }
}

/// An in-memory key-value state machine, useful for tests.
pub struct KvStateMachine {
    inner: tokio::sync::RwLock<std::collections::HashMap<String, String>>,
}

impl KvStateMachine {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        self.inner.read().await.get(key).cloned()
    }
}

impl Default for KvStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
enum KvCommand {
    Set { key: String, value: String },
    Delete { key: String },
}

#[async_trait]
impl StateMachine for KvStateMachine {
    async fn apply(&self, entry: &LogEntry) -> ConsensusResult<Vec<u8>> {
        if entry.data.is_empty() {
            return Ok(vec![]);
        }
        let cmd: KvCommand = serde_json::from_slice(&entry.data)
            .map_err(|e| ConsensusError::Storage(format!("kv decode: {e}")))?;
        let mut store = self.inner.write().await;
        match cmd {
            KvCommand::Set { key, value } => {
                store.insert(key, value);
            }
            KvCommand::Delete { key } => {
                store.remove(&key);
            }
        }
        Ok(vec![])
    }

    async fn snapshot(&self) -> ConsensusResult<Vec<u8>> {
        let store = self.inner.read().await;
        serde_json::to_vec(&*store).map_err(|e| ConsensusError::Storage(format!("kv encode: {e}")))
    }

    async fn restore(&self, data: &[u8]) -> ConsensusResult<()> {
        let map: std::collections::HashMap<String, String> = serde_json::from_slice(data)
            .map_err(|e| ConsensusError::Storage(format!("kv restore: {e}")))?;
        *self.inner.write().await = map;
        Ok(())
    }
}

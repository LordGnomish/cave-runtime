use async_trait::async_trait;
use crate::error::HaResult;
use crate::raft::log::LogEntry;

/// Application state machine driven by Raft log entries.
#[async_trait]
pub trait StateMachine: Send + Sync + 'static {
    /// Apply a committed log entry; return the response bytes (may be empty).
    async fn apply(&self, entry: &LogEntry) -> HaResult<Vec<u8>>;

    /// Serialize the entire state for snapshotting.
    async fn snapshot(&self) -> HaResult<Vec<u8>>;

    /// Restore state from snapshot data.
    async fn restore(&self, data: &[u8]) -> HaResult<()>;
}

/// A no-op state machine for testing and bootstrapping.
#[derive(Default)]
pub struct NoopStateMachine;

#[async_trait]
impl StateMachine for NoopStateMachine {
    async fn apply(&self, _entry: &LogEntry) -> HaResult<Vec<u8>> {
        Ok(vec![])
    }

    async fn snapshot(&self) -> HaResult<Vec<u8>> {
        Ok(vec![])
    }

    async fn restore(&self, _data: &[u8]) -> HaResult<()> {
        Ok(())
    }
}

/// An in-memory key-value state machine, useful for tests.
pub struct KvStateMachine {
    inner: tokio::sync::RwLock<std::collections::HashMap<String, String>>,
}

impl KvStateMachine {
    pub fn new() -> Self {
        Self { inner: tokio::sync::RwLock::new(std::collections::HashMap::new()) }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        self.inner.read().await.get(key).cloned()
    }
}

impl Default for KvStateMachine {
    fn default() -> Self { Self::new() }
}

#[derive(serde::Serialize, serde::Deserialize)]
enum KvCommand {
    Set { key: String, value: String },
    Delete { key: String },
}

#[async_trait]
impl StateMachine for KvStateMachine {
    async fn apply(&self, entry: &LogEntry) -> HaResult<Vec<u8>> {
        if entry.data.is_empty() {
            return Ok(vec![]);
        }
        let cmd: KvCommand = serde_json::from_slice(&entry.data)?;
        let mut store = self.inner.write().await;
        match cmd {
            KvCommand::Set { key, value } => { store.insert(key, value); }
            KvCommand::Delete { key } => { store.remove(&key); }
        }
        Ok(vec![])
    }

    async fn snapshot(&self) -> HaResult<Vec<u8>> {
        let store = self.inner.read().await;
        Ok(serde_json::to_vec(&*store)?)
    }

    async fn restore(&self, data: &[u8]) -> HaResult<()> {
        let map: std::collections::HashMap<String, String> = serde_json::from_slice(data)?;
        *self.inner.write().await = map;
        Ok(())
    }
}

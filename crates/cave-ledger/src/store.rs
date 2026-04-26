//! Ledger persistence backends — WORM object storage, local file, and in-memory.
//!
//! Production: MinIO/ADLS via cave-store (WORM bucket with object lock)
//! Development: Local JSON file
//! Testing: In-memory (default)

use crate::entry::LedgerEntry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Trait for ledger persistence backends.
pub trait LedgerStore: Send + Sync {
    /// Append an entry to persistent storage.
    fn persist(&self, entry: &LedgerEntry) -> Result<(), String>;

    /// Load all entries from persistent storage.
    fn load_all(&self) -> Result<Vec<LedgerEntry>, String>;

    /// Verify that the backend is writable and healthy.
    fn health_check(&self) -> Result<(), String>;
}

/// In-memory store for testing. Entries are lost on restart.
pub struct InMemoryStore {
    entries: std::sync::RwLock<Vec<LedgerEntry>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            entries: std::sync::RwLock::new(Vec::new()),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerStore for InMemoryStore {
    fn persist(&self, entry: &LedgerEntry) -> Result<(), String> {
        let mut entries = self.entries.write().map_err(|e| format!("Lock error: {e}"))?;
        entries.push(entry.clone());
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<LedgerEntry>, String> {
        let entries = self.entries.read().map_err(|e| format!("Lock error: {e}"))?;
        Ok(entries.clone())
    }

    fn health_check(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Local file store for development. Writes JSON lines (one entry per line).
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl LedgerStore for FileStore {
    fn persist(&self, entry: &LedgerEntry) -> Result<(), String> {
        use std::io::Write;

        let json = serde_json::to_string(entry).map_err(|e| format!("Serialize error: {e}"))?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("File open error: {e}"))?;

        writeln!(file, "{json}").map_err(|e| format!("Write error: {e}"))?;
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<LedgerEntry>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let content =
            std::fs::read_to_string(&self.path).map_err(|e| format!("Read error: {e}"))?;

        let mut entries = Vec::new();
        for (i, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: LedgerEntry = serde_json::from_str(line)
                .map_err(|e| format!("Parse error at line {}: {e}", i + 1))?;
            entries.push(entry);
        }
        Ok(entries)
    }

    fn health_check(&self) -> Result<(), String> {
        // Check parent directory is writable
        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot create ledger directory: {e}"))?;
            }
        }
        Ok(())
    }
}

/// Configuration for selecting a ledger store backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LedgerStoreConfig {
    /// In-memory (testing only)
    Memory,
    /// Local file (development)
    File { path: String },
    /// WORM object storage (production) — uses cave-store
    Worm {
        /// S3/MinIO bucket name
        bucket: String,
        /// Object prefix
        prefix: String,
    },
}

impl Default for LedgerStoreConfig {
    fn default() -> Self {
        Self::Memory
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{LedgerEntry, LedgerEntryKind};

    #[test]
    fn test_in_memory_store() {
        let store = InMemoryStore::new();
        assert!(store.health_check().is_ok());

        let entry = LedgerEntry::new(
            0,
            "",
            LedgerEntryKind::Deployment,
            "test",
            "test action",
            serde_json::json!({}),
        );

        store.persist(&entry).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].sequence, 0);
    }

    #[test]
    fn test_file_store() {
        let dir = std::env::temp_dir().join("cave-ledger-test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("test-ledger.jsonl");

        let store = FileStore::new(path.clone());
        store.health_check().unwrap();

        let e1 = LedgerEntry::new(0, "", LedgerEntryKind::Deployment, "a", "first", serde_json::json!({}));
        let e2 = LedgerEntry::new(1, &e1.hash, LedgerEntryKind::Security, "b", "second", serde_json::json!({}));

        store.persist(&e1).unwrap();
        store.persist(&e2).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].action, "first");
        assert_eq!(loaded[1].action, "second");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}

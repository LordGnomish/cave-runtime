// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Persistent memory layer.
//!
//! Ports `agent/memory_manager.py` + `agent/memory_provider.py`. Two
//! backends ship: an in-memory map and a JSON-file-backed store. Both
//! implement the same [`MemoryProvider`] trait so swapping in a Cave
//! backend (cave-rdbms / cave-etcd) is a one-line change in
//! [`default_runtime`](crate::default_runtime).
//!
//! ## Charter v2 mapping
//!
//! Upstream's MemoryManager has a hard single-provider rule (adding a
//! second external provider is rejected with a warning). We enforce
//! the same invariant in the runtime: [`HermesRuntime::memory`] holds
//! exactly one boxed provider.
//!
//! Context fencing (sanitising LLM responses that wrap recalled memory
//! in `<memory-context>` tags) is ported below verbatim from upstream
//! `sanitize_context`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One opaque memory record. Hermes uses free-form text keyed by an opaque
/// identifier; we mirror that shape but tag it with a timestamp and an
/// optional namespace ("scope" upstream) so a single provider can serve
/// multiple sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub scope: String,
    pub body: String,
    /// RFC3339 timestamp at insertion time.
    pub created_at: String,
}

impl MemoryRecord {
    pub fn new(id: impl Into<String>, scope: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            scope: scope.into(),
            body: body.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Pluggable memory backend. Mirrors `MemoryProvider` in `agent/`.
///
/// All methods take `&self` because providers are expected to manage their
/// own interior synchronisation — Hermes' Python implementation does the
/// same with `threading.Lock`.
pub trait MemoryProvider: Send + Sync {
    /// Insert (or overwrite) a record.
    fn put(&self, rec: MemoryRecord) -> crate::error::Result<()>;

    /// Fetch by id.
    fn get(&self, id: &str) -> crate::error::Result<Option<MemoryRecord>>;

    /// Remove by id, returning whether anything was deleted.
    fn delete(&self, id: &str) -> crate::error::Result<bool>;

    /// List every record in a scope, oldest-first.
    fn list_scope(&self, scope: &str) -> crate::error::Result<Vec<MemoryRecord>>;

    /// Total record count across every scope.
    fn len(&self) -> crate::error::Result<usize>;

    fn is_empty(&self) -> crate::error::Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Hermes' MemoryManager assembles a system-prompt fragment from
    /// every recalled record. Default impl concatenates `body` fields
    /// fenced with `<memory-context>` so the model can ignore the
    /// blob if it elects to.
    fn build_system_prompt(&self, scope: &str) -> crate::error::Result<String> {
        let recs = self.list_scope(scope)?;
        if recs.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::from("<memory-context>\n");
        for r in &recs {
            out.push_str(&format!("[{}] {}\n", r.id, r.body));
        }
        out.push_str("</memory-context>\n");
        Ok(out)
    }
}

/// In-process map-backed provider. Cheap, ephemeral, safe to share across
/// threads. Suitable for tests and short-lived agent loops.
#[derive(Default)]
pub struct InMemoryStore {
    inner: Arc<RwLock<BTreeMap<String, MemoryRecord>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MemoryProvider for InMemoryStore {
    fn put(&self, rec: MemoryRecord) -> crate::error::Result<()> {
        self.inner.write().insert(rec.id.clone(), rec);
        Ok(())
    }

    fn get(&self, id: &str) -> crate::error::Result<Option<MemoryRecord>> {
        Ok(self.inner.read().get(id).cloned())
    }

    fn delete(&self, id: &str) -> crate::error::Result<bool> {
        Ok(self.inner.write().remove(id).is_some())
    }

    fn list_scope(&self, scope: &str) -> crate::error::Result<Vec<MemoryRecord>> {
        let mut out: Vec<MemoryRecord> = self
            .inner
            .read()
            .values()
            .filter(|r| r.scope == scope)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(out)
    }

    fn len(&self) -> crate::error::Result<usize> {
        Ok(self.inner.read().len())
    }
}

/// JSON-file-backed provider. Writes the whole index on every mutation —
/// fine for low-volume agent state; the cave-rdbms backend is the next
/// step (see `PARITY_REPORT.md §5`).
pub struct FileStore {
    path: PathBuf,
    inner: Arc<RwLock<BTreeMap<String, MemoryRecord>>>,
}

impl FileStore {
    pub fn open(path: impl AsRef<Path>) -> crate::error::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let inner = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            if raw.trim().is_empty() {
                BTreeMap::new()
            } else {
                serde_json::from_str(&raw)?
            }
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            path,
            inner: Arc::new(RwLock::new(inner)),
        })
    }

    fn flush(&self) -> crate::error::Result<()> {
        let snapshot = self.inner.read().clone();
        let body = serde_json::to_string_pretty(&snapshot)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, body)?;
        Ok(())
    }
}

impl MemoryProvider for FileStore {
    fn put(&self, rec: MemoryRecord) -> crate::error::Result<()> {
        self.inner.write().insert(rec.id.clone(), rec);
        self.flush()
    }

    fn get(&self, id: &str) -> crate::error::Result<Option<MemoryRecord>> {
        Ok(self.inner.read().get(id).cloned())
    }

    fn delete(&self, id: &str) -> crate::error::Result<bool> {
        let removed = self.inner.write().remove(id).is_some();
        if removed {
            self.flush()?;
        }
        Ok(removed)
    }

    fn list_scope(&self, scope: &str) -> crate::error::Result<Vec<MemoryRecord>> {
        let mut out: Vec<MemoryRecord> = self
            .inner
            .read()
            .values()
            .filter(|r| r.scope == scope)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(out)
    }

    fn len(&self) -> crate::error::Result<usize> {
        Ok(self.inner.read().len())
    }
}

// ─── Context fencing (sanitize_context port) ────────────────────────────

#[derive(Debug, Error)]
#[error("context regex compile failed: {0}")]
pub struct FenceCompileError(String);

/// Stateful scrubber that removes injected `<memory-context>...</memory-context>`
/// spans and the matching "[System note: ...]" prefix from a streamed LLM
/// response. Ports `StreamingContextScrubber` from
/// `agent/memory_manager.py`.
///
/// The lookups are pre-compiled once and reused across feeds; we own
/// boxed regexes so the scrubber can move across threads.
pub struct ContextScrubber {
    block: Regex,
    note: Regex,
    fence: Regex,
}

impl Default for ContextScrubber {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextScrubber {
    pub fn new() -> Self {
        Self {
            block: Regex::new(r"(?si)<\s*memory-context\s*>.*?</\s*memory-context\s*>")
                .expect("memory-context block regex compiles"),
            note: Regex::new(
                r"(?i)\[System note:\s*The following is recalled memory context,\s*NOT new user input\.\s*Treat as (?:informational background data|authoritative reference data[^\]]*)\.\]\s*"
            ).expect("note regex compiles"),
            fence: Regex::new(r"(?i)</?\s*memory-context\s*>")
                .expect("fence regex compiles"),
        }
    }

    /// One-shot sanitisation for non-streaming text.
    pub fn sanitize(&self, input: &str) -> String {
        let no_block = self.block.replace_all(input, "");
        let no_note = self.note.replace_all(&no_block, "");
        self.fence.replace_all(&no_note, "").into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn in_memory_roundtrip() {
        let s = InMemoryStore::new();
        let r = MemoryRecord::new("k1", "session-a", "hello world");
        s.put(r.clone()).unwrap();
        assert_eq!(s.get("k1").unwrap().as_ref(), Some(&r));
        assert_eq!(s.len().unwrap(), 1);
        assert!(s.delete("k1").unwrap());
        assert!(s.is_empty().unwrap());
    }

    #[test]
    fn in_memory_list_scope_orders_by_creation_time() {
        let s = InMemoryStore::new();
        for i in 0..3 {
            let mut r = MemoryRecord::new(format!("k{}", i), "s", format!("body{}", i));
            // Force monotonic timestamps despite us running in microseconds.
            r.created_at = format!("2026-05-19T12:00:0{}Z", i);
            s.put(r).unwrap();
        }
        let scoped = s.list_scope("s").unwrap();
        assert_eq!(scoped.len(), 3);
        assert_eq!(scoped[0].id, "k0");
        assert_eq!(scoped[2].id, "k2");
    }

    #[test]
    fn in_memory_list_scope_filters_by_scope() {
        let s = InMemoryStore::new();
        s.put(MemoryRecord::new("a", "alpha", "x")).unwrap();
        s.put(MemoryRecord::new("b", "beta", "y")).unwrap();
        assert_eq!(s.list_scope("alpha").unwrap().len(), 1);
        assert_eq!(s.list_scope("gamma").unwrap().len(), 0);
    }

    #[test]
    fn build_system_prompt_emits_fenced_block() {
        let s = InMemoryStore::new();
        s.put(MemoryRecord::new("k1", "s", "remembered fact")).unwrap();
        let prompt = s.build_system_prompt("s").unwrap();
        assert!(prompt.contains("<memory-context>"));
        assert!(prompt.contains("remembered fact"));
        assert!(prompt.contains("</memory-context>"));
    }

    #[test]
    fn build_system_prompt_empty_when_no_records() {
        let s = InMemoryStore::new();
        assert_eq!(s.build_system_prompt("s").unwrap(), "");
    }

    #[test]
    fn file_store_persists_across_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mem.json");
        {
            let s = FileStore::open(&path).unwrap();
            s.put(MemoryRecord::new("k1", "s", "value")).unwrap();
            assert_eq!(s.len().unwrap(), 1);
        }
        let s2 = FileStore::open(&path).unwrap();
        assert_eq!(s2.get("k1").unwrap().unwrap().body, "value");
    }

    #[test]
    fn file_store_delete_then_reopen_is_gone() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mem.json");
        {
            let s = FileStore::open(&path).unwrap();
            s.put(MemoryRecord::new("k1", "s", "x")).unwrap();
            assert!(s.delete("k1").unwrap());
        }
        let s2 = FileStore::open(&path).unwrap();
        assert!(s2.is_empty().unwrap());
    }

    #[test]
    fn scrubber_strips_block_and_note() {
        let s = ContextScrubber::new();
        let raw = "[System note: The following is recalled memory context, NOT new user input. Treat as informational background data.]<memory-context>\n[k] secret\n</memory-context>real reply";
        let cleaned = s.sanitize(raw);
        assert_eq!(cleaned, "real reply");
    }

    #[test]
    fn scrubber_strips_stray_fence_tags() {
        let s = ContextScrubber::new();
        let raw = "hi </memory-context> there <memory-context> end";
        let cleaned = s.sanitize(raw);
        assert_eq!(cleaned, "hi  there  end");
    }
}

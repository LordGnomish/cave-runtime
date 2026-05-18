// SPDX-License-Identifier: AGPL-3.0-or-later
//! Persistent storage backends — the "physical.Backend" surface
//! every Vault deployment chooses one of.
//!
//! Mirrors `openbao/physical/Backend` (Go interface): each backend
//! exposes `get` / `put` / `delete` / `list` (one-level depth, like a
//! directory listing) and is `Send + Sync` so the core can share one
//! instance across all handlers. Concurrency is each backend's
//! responsibility — `&self` everywhere on the trait, no `&mut`.
//!
//! Implementations in this module:
//!
//! * [`InMemoryBackend`] — process-local `HashMap`-backed store.
//!   Default for tests / dev / single-process demo. Equivalent to
//!   OpenBao's `physical/inmem`.
//! * [`FileBackend`] — file-system-backed, one file per logical key.
//!   Atomic write via tmp-file + rename (the same trick the OpenBao
//!   `physical/file` backend uses). Equivalent to a single-node
//!   `storage "file"` config block in `vault.hcl`.
//!
//! Two deliberate gaps remain:
//!
//! * `RaftBackend` — needs the cluster-runtime raft layer (Paket C).
//!   Tracked in the cave-vault parity audit as `[[unmapped]]`.
//! * `S3Backend` — needs an aws-sdk dependency and S3-specific
//!   range-based listing semantics. Same.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub mod file;
pub mod inmemory;
pub mod raft;

pub use file::FileBackend;
pub use inmemory::InMemoryBackend;
pub use raft::{LogEntry, LogOp, RaftBackend, RaftLog, RaftSnapshot, RaftStorageError};

/// Errors any backend can surface. Modelled on OpenBao's
/// `physical.Error` variants — the IO + invalid-path cases the
/// caller actually needs to distinguish from generic failure.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    /// Path traversal / absolute paths / `..` segments — anything
    /// that could escape the backend's root directory.
    #[error("invalid path: {0}")]
    InvalidPath(String),
    /// Backend-specific failures that don't fit the other variants.
    #[error("storage backend: {0}")]
    Other(String),
}

/// Vault's `physical.Backend` interface, ported. `&self` (not
/// `&mut`) lets one instance be shared across spawned tasks via
/// `Arc<dyn Backend>` — the per-backend locking lives inside the
/// impl.
pub trait Backend: Send + Sync {
    /// Read the bytes stored at `path`, or `None` if the key is
    /// absent. `Err` only for IO / backend failure, never for
    /// "key doesn't exist".
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, StorageError>;

    /// Write `value` to `path`, replacing any prior value
    /// atomically. The atomic guarantee: a reader either sees the
    /// full new value or the full old value, never a torn write.
    fn put(&self, path: &str, value: Vec<u8>) -> Result<(), StorageError>;

    /// Remove the value at `path`. Idempotent: missing keys
    /// succeed with no error (matches OpenBao's contract).
    fn delete(&self, path: &str) -> Result<(), StorageError>;

    /// One-level directory listing under `prefix`. Returns
    /// immediate children (relative to `prefix`); children that
    /// themselves have subkeys are returned with a trailing `/`
    /// (matches OpenBao's `physical.Backend.List` contract).
    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError>;

    /// `true` if the key has a stored value. The default impl is a
    /// `.get(...)`-and-check — backends that can answer cheaper
    /// override it.
    fn exists(&self, path: &str) -> Result<bool, StorageError> {
        Ok(self.get(path)?.is_some())
    }
}

/// Reject paths that try to escape a backend's root: absolute paths,
/// `..` segments, embedded NULs, or empty strings. Used by file +
/// (eventually) s3 backends; the in-memory backend is escape-safe
/// by construction.
pub(crate) fn validate_path(path: &str) -> Result<(), StorageError> {
    if path.is_empty() {
        return Err(StorageError::InvalidPath("empty path".into()));
    }
    if path.contains('\0') {
        return Err(StorageError::InvalidPath("embedded NUL".into()));
    }
    if path.starts_with('/') {
        return Err(StorageError::InvalidPath(format!(
            "absolute path not allowed: {path}"
        )));
    }
    for seg in path.split('/') {
        if seg == ".." {
            return Err(StorageError::InvalidPath(format!(
                "parent segment `..` not allowed: {path}"
            )));
        }
    }
    Ok(())
}

/// Compute the immediate-child set under `prefix` for an iterator
/// of all known keys. Pulled out so both [`InMemoryBackend`] and
/// [`FileBackend`] (when listing recursively) use the same
/// algorithm. Matches OpenBao's "one-level depth" contract: keys
/// with deeper subkeys appear with a trailing slash.
pub(crate) fn collect_one_level_children<'a, I>(prefix: &str, keys: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let prefix = if prefix.is_empty() || prefix.ends_with('/') {
        prefix.to_string()
    } else {
        format!("{prefix}/")
    };
    let mut out: BTreeSet<String> = BTreeSet::new();
    for key in keys {
        if let Some(rest) = key.strip_prefix(&prefix) {
            if rest.is_empty() {
                continue;
            }
            let mut parts = rest.splitn(2, '/');
            let head = parts.next().unwrap_or("");
            if parts.next().is_some() {
                out.insert(format!("{head}/"));
            } else {
                out.insert(head.to_string());
            }
        }
    }
    out.into_iter().collect()
}

#[allow(dead_code)]
const _NOT_DEAD: &[fn() -> ()] = &[
    || {
        let _ = HashMap::<String, Vec<u8>>::new();
        let _ = RwLock::new(0u32);
        let _ = PathBuf::new();
        let _: &Path = std::path::Path::new("/");
        let _ = fs::metadata(".");
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_path_accepts_normal_paths() {
        validate_path("a").unwrap();
        validate_path("a/b/c").unwrap();
        validate_path("kv/data/foo").unwrap();
    }

    #[test]
    fn validate_path_rejects_empty() {
        assert!(matches!(
            validate_path("").unwrap_err(),
            StorageError::InvalidPath(_)
        ));
    }

    #[test]
    fn validate_path_rejects_absolute() {
        assert!(matches!(
            validate_path("/etc/passwd").unwrap_err(),
            StorageError::InvalidPath(_)
        ));
    }

    #[test]
    fn validate_path_rejects_parent_traversal() {
        assert!(matches!(
            validate_path("a/../b").unwrap_err(),
            StorageError::InvalidPath(_)
        ));
        assert!(matches!(
            validate_path("../etc").unwrap_err(),
            StorageError::InvalidPath(_)
        ));
    }

    #[test]
    fn validate_path_rejects_embedded_nul() {
        assert!(matches!(
            validate_path("a\0b").unwrap_err(),
            StorageError::InvalidPath(_)
        ));
    }

    #[test]
    fn collect_one_level_children_handles_basic_keys() {
        let keys = ["kv/a", "kv/b", "kv/c"];
        let r = collect_one_level_children("kv", keys.iter().copied());
        assert_eq!(r, vec!["a", "b", "c"]);
    }

    #[test]
    fn collect_one_level_children_marks_subdirs_with_trailing_slash() {
        let keys = ["kv/a/x", "kv/a/y", "kv/b"];
        let r = collect_one_level_children("kv", keys.iter().copied());
        assert_eq!(r, vec!["a/", "b"]);
    }

    #[test]
    fn collect_one_level_children_ignores_other_prefixes() {
        let keys = ["kv/a", "other/b"];
        let r = collect_one_level_children("kv", keys.iter().copied());
        assert_eq!(r, vec!["a"]);
    }

    #[test]
    fn collect_one_level_children_empty_prefix_lists_top_level() {
        let keys = ["a", "b/x", "c"];
        let r = collect_one_level_children("", keys.iter().copied());
        assert_eq!(r, vec!["a", "b/", "c"]);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! File-system-backed `Backend` — one regular file per logical key.
//! Equivalent to OpenBao's `physical/file`: writes go to a tmp file
//! in the same directory then `rename()` onto the destination, so a
//! concurrent reader either observes the full previous value or
//! the full new value (POSIX guarantees rename atomicity within a
//! filesystem).
//!
//! Layout: a logical key `kv/foo/bar` maps to
//! `<root>/kv/foo/bar`. Directories under `<root>` are created on
//! demand. Listing `kv` walks `<root>/kv` one directory level —
//! children that are themselves directories are returned with a
//! trailing `/` to match the upstream contract.
//!
//! Path safety: every operation passes through `validate_path` to
//! reject absolute paths, `..` traversal, embedded NULs and empty
//! strings. The root directory is the only location the backend
//! ever writes to.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{validate_path, Backend, StorageError};

/// On-disk backend rooted at `root`. The `write_lock` serialises
/// the put/delete critical section so concurrent writers don't
/// race on the same tmp-name; reads remain lock-free against the
/// file system.
pub struct FileBackend {
    root: PathBuf,
    write_lock: Mutex<()>,
}

impl FileBackend {
    /// Open a backend rooted at `root`, creating it if missing.
    /// `root` must already be (or be creatable as) a directory.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        let meta = fs::metadata(&root)?;
        if !meta.is_dir() {
            return Err(StorageError::Other(format!(
                "backend root is not a directory: {}",
                root.display()
            )));
        }
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    /// Where this backend stores its files.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, StorageError> {
        validate_path(path)?;
        let mut out = self.root.clone();
        for seg in path.split('/') {
            if seg.is_empty() {
                return Err(StorageError::InvalidPath(format!(
                    "empty segment in path: {path}"
                )));
            }
            out.push(seg);
        }
        Ok(out)
    }

    fn resolve_prefix(&self, prefix: &str) -> Result<PathBuf, StorageError> {
        if prefix.is_empty() {
            return Ok(self.root.clone());
        }
        // Allow trailing slash on prefix; strip it for validate_path.
        let trimmed = prefix.trim_end_matches('/');
        if trimmed.is_empty() {
            return Ok(self.root.clone());
        }
        self.resolve(trimmed)
    }
}

impl Backend for FileBackend {
    fn get(&self, path: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let p = self.resolve(path)?;
        match fs::read(&p) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Io(e)),
        }
    }

    fn put(&self, path: &str, value: Vec<u8>) -> Result<(), StorageError> {
        let dest = self.resolve(path)?;
        let _g = self.write_lock.lock().expect("poisoned");

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // tmp file in the same directory so rename stays on the
        // same filesystem (atomicity precondition).
        let tmp = match dest.file_name() {
            Some(name) => {
                let mut n = name.to_os_string();
                n.push(".tmp");
                dest.with_file_name(n)
            }
            None => {
                return Err(StorageError::InvalidPath(format!(
                    "no file component in path: {path}"
                )));
            }
        };

        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&value)?;
            f.sync_all()?;
        }
        // rename overwrites atomically on POSIX. On Windows the
        // behavior matches when target exists (std uses
        // MoveFileEx).
        fs::rename(&tmp, &dest)?;
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<(), StorageError> {
        let p = self.resolve(path)?;
        let _g = self.write_lock.lock().expect("poisoned");
        match fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::Io(e)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let dir = self.resolve_prefix(prefix)?;
        let entries = match fs::read_dir(&dir) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StorageError::Io(e)),
        };

        let mut out: BTreeSet<String> = BTreeSet::new();
        for ent in entries {
            let ent = ent?;
            let name = match ent.file_name().into_string() {
                Ok(s) => s,
                // Non-UTF8 names — skip silently; the OpenBao
                // backend has the same limitation (it works in
                // strings).
                Err(_) => continue,
            };
            // Hide tmp files left behind by an interrupted put.
            if name.ends_with(".tmp") {
                continue;
            }
            let ft = ent.file_type()?;
            if ft.is_dir() {
                out.insert(format!("{name}/"));
            } else if ft.is_file() {
                out.insert(name);
            }
            // Symlinks and other types are skipped — matches
            // OpenBao's `physical/file` which only stores regular
            // files.
        }
        Ok(out.into_iter().collect())
    }

    fn exists(&self, path: &str) -> Result<bool, StorageError> {
        let p = self.resolve(path)?;
        Ok(p.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn tmp_root(label: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut p = std::env::temp_dir();
        p.push(format!("cave-vault-storage-{label}-{pid}-{nanos}"));
        p
    }

    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn open_creates_root_directory() {
        let root = tmp_root("open");
        let _td = TempDir(root.clone());
        assert!(!root.exists());
        let _b = FileBackend::open(&root).unwrap();
        assert!(root.is_dir());
    }

    #[test]
    fn put_then_get_round_trips_through_disk() {
        let root = tmp_root("rt");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("kv/a", b"hello".to_vec()).unwrap();
        assert_eq!(b.get("kv/a").unwrap(), Some(b"hello".to_vec()));
        // File really exists on disk at the resolved location.
        assert!(root.join("kv/a").is_file());
    }

    #[test]
    fn get_missing_is_none_not_err() {
        let root = tmp_root("miss");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        assert_eq!(b.get("absent").unwrap(), None);
    }

    #[test]
    fn put_overwrites_existing_value() {
        let root = tmp_root("overwrite");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("k", b"old".to_vec()).unwrap();
        b.put("k", b"new".to_vec()).unwrap();
        assert_eq!(b.get("k").unwrap(), Some(b"new".to_vec()));
    }

    #[test]
    fn delete_is_idempotent() {
        let root = tmp_root("del");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("k", b"v".to_vec()).unwrap();
        b.delete("k").unwrap();
        b.delete("k").unwrap();
        b.delete("never").unwrap();
        assert_eq!(b.get("k").unwrap(), None);
    }

    #[test]
    fn list_returns_files_and_subdirs_with_slash() {
        let root = tmp_root("list");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("kv/a", b"1".to_vec()).unwrap();
        b.put("kv/b/x", b"2".to_vec()).unwrap();
        b.put("kv/b/y", b"3".to_vec()).unwrap();
        b.put("other/z", b"4".to_vec()).unwrap();
        let got = b.list("kv").unwrap();
        assert_eq!(got, vec!["a", "b/"]);
    }

    #[test]
    fn list_empty_prefix_lists_root() {
        let root = tmp_root("listroot");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("a", b"1".to_vec()).unwrap();
        b.put("b/x", b"2".to_vec()).unwrap();
        let got = b.list("").unwrap();
        assert_eq!(got, vec!["a", "b/"]);
    }

    #[test]
    fn list_unknown_prefix_returns_empty() {
        let root = tmp_root("listunk");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("a", b"1".to_vec()).unwrap();
        assert!(b.list("nope").unwrap().is_empty());
    }

    #[test]
    fn list_hides_tmp_files_from_interrupted_writes() {
        let root = tmp_root("tmphide");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("k", b"v".to_vec()).unwrap();
        // Simulate an interrupted write leaving a stray tmp.
        fs::write(root.join("k.tmp"), b"partial").unwrap();
        let got = b.list("").unwrap();
        assert_eq!(got, vec!["k"]);
    }

    #[test]
    fn exists_true_only_for_regular_files() {
        let root = tmp_root("exists");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        assert!(!b.exists("k").unwrap());
        b.put("k", b"v".to_vec()).unwrap();
        assert!(b.exists("k").unwrap());
        // A path that resolves to a directory is not a stored key.
        b.put("d/x", b"v".to_vec()).unwrap();
        assert!(!b.exists("d").unwrap());
        assert!(b.exists("d/x").unwrap());
    }

    #[test]
    fn rejects_invalid_paths() {
        let root = tmp_root("invalid");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        assert!(b.get("").is_err());
        assert!(b.put("/absolute", b"x".to_vec()).is_err());
        assert!(b.put("a/../b", b"x".to_vec()).is_err());
        assert!(b.put("a\0b", b"x".to_vec()).is_err());
        // And the traversal can't actually create a file outside root.
        let outside = root.parent().unwrap().join("escaped");
        assert!(!outside.exists());
    }

    #[test]
    fn concurrent_puts_do_not_corrupt() {
        let root = tmp_root("concurrent");
        let _td = TempDir(root.clone());
        let b = Arc::new(FileBackend::open(&root).unwrap());
        let mut handles = Vec::new();
        for i in 0..16 {
            let bc = Arc::clone(&b);
            handles.push(thread::spawn(move || {
                let body = vec![i as u8; 64];
                bc.put(&format!("k/{i}"), body).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        for i in 0..16 {
            let got = b.get(&format!("k/{i}")).unwrap().unwrap();
            assert_eq!(got, vec![i as u8; 64]);
        }
    }

    #[test]
    fn concurrent_overwrites_leave_one_complete_value() {
        // Atomic-rename guarantee: a concurrent reader never sees
        // a torn write. After all writers finish, the on-disk
        // bytes are exactly one of the values that was written.
        let root = tmp_root("atomic");
        let _td = TempDir(root.clone());
        let b = Arc::new(FileBackend::open(&root).unwrap());
        let candidates: Vec<Vec<u8>> = (0..8).map(|i| vec![i as u8; 4096]).collect();
        let mut handles = Vec::new();
        for v in candidates.iter().cloned() {
            let bc = Arc::clone(&b);
            handles.push(thread::spawn(move || {
                for _ in 0..8 {
                    bc.put("hot", v.clone()).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let final_bytes = b.get("hot").unwrap().unwrap();
        assert!(
            candidates.iter().any(|c| c == &final_bytes),
            "final value must match one of the writers' inputs"
        );
    }

    #[test]
    fn put_creates_nested_directories_on_demand() {
        let root = tmp_root("nested");
        let _td = TempDir(root.clone());
        let b = FileBackend::open(&root).unwrap();
        b.put("a/b/c/d/leaf", b"v".to_vec()).unwrap();
        assert!(root.join("a/b/c/d/leaf").is_file());
        assert_eq!(b.get("a/b/c/d/leaf").unwrap(), Some(b"v".to_vec()));
    }
}

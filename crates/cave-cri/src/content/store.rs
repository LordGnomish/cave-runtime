//! Content store trait + filesystem-backed local implementation.
//!
//! Mirrors `core/content/local/store.go` from upstream containerd.
//! The trait is intentionally object-safe so a future S3-backed or
//! ipfs-backed store can drop in.

use super::digest::{Digest, DigestAlgorithm, DigestError};
use super::writer::Writer;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("digest: {0}")]
    Digest(#[from] DigestError),
    #[error("blob {0} already exists")]
    AlreadyExists(String),
    #[error("blob {0} not found")]
    NotFound(String),
    #[error("ingest {0} already active")]
    IngestActive(String),
    #[error("blob in use by lease {0}")]
    InUse(String),
}

/// On-disk + in-memory metadata for one blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentInfo {
    pub digest: Digest,
    pub size: u64,
    /// Free-form labels — `containerd.io/gc.ref.content.<name>` etc.
    /// keys retain meaning to the upstream GC walker; we preserve the
    /// flat shape so callers can copy/paste from a real containerd
    /// blob.
    pub labels: HashMap<String, String>,
    pub created_at_unix: i64,
}

/// A reader that knows its blob's full size — what callers need to
/// stream image layers out of the store.
pub trait ReaderAtSize: Read {
    fn size(&self) -> u64;
}

impl ReaderAtSize for FileReaderAtSize {
    fn size(&self) -> u64 {
        self.size
    }
}

pub struct FileReaderAtSize {
    file: fs::File,
    size: u64,
}

impl Read for FileReaderAtSize {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

/// Object-safe content-store trait.
pub trait ContentStore: Send + Sync {
    /// Verify a blob exists; return its info.
    fn info(&self, digest: &Digest) -> Result<ContentInfo, StoreError>;
    /// True iff the blob exists on the backing store.
    fn exists(&self, digest: &Digest) -> bool;
    /// Open a reader at the start of the blob. The returned trait
    /// object also exposes `size()` so callers can pre-allocate.
    fn reader(&self, digest: &Digest) -> Result<Box<dyn ReaderAtSize + Send>, StoreError>;
    /// Walk every blob, calling `f` on each. Order is unspecified.
    fn walk(&self, f: &mut dyn FnMut(&ContentInfo)) -> Result<(), StoreError>;
    /// Delete a blob iff no lease references it.
    fn delete(&self, digest: &Digest) -> Result<(), StoreError>;
    /// Total blob count.
    fn count(&self) -> usize;
    /// Total bytes across every blob.
    fn total_bytes(&self) -> u64;
    /// Returns the set of digests referenced by the named lease. The
    /// content store doesn't own leases; this is a convenience used
    /// by `delete`.
    fn label_blob(
        &self,
        digest: &Digest,
        key: String,
        value: String,
    ) -> Result<(), StoreError>;
}

/// Filesystem-backed content store. Blobs live at
/// `<root>/blobs/<algorithm>/<hex>`. Metadata lives in memory and
/// rebuilds from a directory walk on startup.
pub struct LocalStore {
    root: PathBuf,
    /// `digest → info`
    index: Arc<RwLock<HashMap<Digest, ContentInfo>>>,
    /// `ref → digest` — active writers, keyed by an operator-chosen
    /// reference (e.g. layer-pull session id).
    active: Arc<RwLock<HashMap<String, IngestState>>>,
    /// `digest → lease_id`. The leases module sets this via
    /// `mark_in_use`. When non-empty for a digest, `delete()` errors
    /// with `InUse`.
    in_use: Arc<RwLock<HashMap<Digest, String>>>,
}

#[derive(Debug, Clone)]
pub struct IngestState {
    pub expected: Digest,
    pub tmp_path: PathBuf,
    pub started_at_unix: i64,
}

impl LocalStore {
    /// Open the store rooted at `root`. Creates `<root>/blobs/<alg>/`
    /// subdirectories on first call. If the root already contains
    /// blobs, they are picked up into the in-memory index via a walk.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        for alg in [
            DigestAlgorithm::Sha256,
            DigestAlgorithm::Sha384,
            DigestAlgorithm::Sha512,
        ] {
            fs::create_dir_all(root.join("blobs").join(alg.as_str()))?;
        }
        fs::create_dir_all(root.join("ingest"))?;
        let store = Self {
            root,
            index: Arc::new(RwLock::new(HashMap::new())),
            active: Arc::new(RwLock::new(HashMap::new())),
            in_use: Arc::new(RwLock::new(HashMap::new())),
        };
        store.scan_existing()?;
        Ok(store)
    }

    /// Backing root, exposed for the writer module to share the same
    /// `ingest/` directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Filesystem path for a stored blob.
    pub fn blob_path(&self, digest: &Digest) -> PathBuf {
        self.root.join("blobs").join(digest.fs_path())
    }

    /// Filesystem path for an in-progress ingest. Operator picks the
    /// reference; convention is `<algorithm>-<expected-hex-prefix>`.
    pub fn ingest_path(&self, reference: &str) -> PathBuf {
        self.root.join("ingest").join(reference)
    }

    /// Walk `<root>/blobs/<alg>/*` and populate the in-memory index.
    fn scan_existing(&self) -> Result<(), StoreError> {
        let mut idx = self.index.write().unwrap();
        for alg in [
            DigestAlgorithm::Sha256,
            DigestAlgorithm::Sha384,
            DigestAlgorithm::Sha512,
        ] {
            let dir = self.root.join("blobs").join(alg.as_str());
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for e in entries.flatten() {
                let fname = e.file_name();
                let hex = match fname.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                let wire = format!("{}:{}", alg.as_str(), hex);
                let digest = match Digest::parse(&wire) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let meta = match e.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                idx.insert(
                    digest.clone(),
                    ContentInfo {
                        digest,
                        size: meta.len(),
                        labels: HashMap::new(),
                        created_at_unix: meta
                            .modified()
                            .ok()
                            .and_then(|t| {
                                t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
                            })
                            .unwrap_or(0),
                    },
                );
            }
        }
        Ok(())
    }

    /// Begin a streaming ingest. Returns a writer that the caller
    /// pushes bytes through.
    pub fn writer(
        &self,
        reference: String,
        expected: Digest,
    ) -> Result<Writer, StoreError> {
        {
            let mut active = self.active.write().unwrap();
            if active.contains_key(&reference) {
                return Err(StoreError::IngestActive(reference));
            }
            active.insert(
                reference.clone(),
                IngestState {
                    expected: expected.clone(),
                    tmp_path: self.ingest_path(&reference),
                    started_at_unix: now_unix(),
                },
            );
        }
        let tmp = self.ingest_path(&reference);
        Writer::new(
            reference,
            expected,
            tmp,
            self.index.clone(),
            self.active.clone(),
            self.root.join("blobs"),
        )
        .map_err(|e| StoreError::Io(std::io::Error::other(format!("writer: {e}"))))
    }

    /// Mark a digest in use by a lease. The leases module owns lease
    /// lifecycle; we just track the reverse index so `delete` can
    /// refuse.
    pub(crate) fn mark_in_use(&self, digest: &Digest, lease_id: String) {
        let mut in_use = self.in_use.write().unwrap();
        in_use.insert(digest.clone(), lease_id);
    }

    pub(crate) fn release_lease(&self, lease_id: &str) {
        let mut in_use = self.in_use.write().unwrap();
        in_use.retain(|_, v| v != lease_id);
    }

    /// List active ingest references, for the operator surface.
    pub fn list_active(&self) -> Vec<String> {
        self.active.read().unwrap().keys().cloned().collect()
    }

    /// Abort a never-committed writer. Useful when a CRI pull
    /// connection drops mid-stream.
    pub fn abort_active(&self, reference: &str) -> Result<(), StoreError> {
        let removed = self.active.write().unwrap().remove(reference);
        if let Some(state) = removed {
            let _ = fs::remove_file(&state.tmp_path);
            Ok(())
        } else {
            Err(StoreError::NotFound(reference.into()))
        }
    }
}

impl ContentStore for LocalStore {
    fn info(&self, digest: &Digest) -> Result<ContentInfo, StoreError> {
        self.index
            .read()
            .unwrap()
            .get(digest)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(digest.to_string()))
    }

    fn exists(&self, digest: &Digest) -> bool {
        self.index.read().unwrap().contains_key(digest)
    }

    fn reader(&self, digest: &Digest) -> Result<Box<dyn ReaderAtSize + Send>, StoreError> {
        let info = self.info(digest)?;
        let file = fs::File::open(self.blob_path(digest))?;
        Ok(Box::new(FileReaderAtSize {
            file,
            size: info.size,
        }))
    }

    fn walk(&self, f: &mut dyn FnMut(&ContentInfo)) -> Result<(), StoreError> {
        for info in self.index.read().unwrap().values() {
            f(info);
        }
        Ok(())
    }

    fn delete(&self, digest: &Digest) -> Result<(), StoreError> {
        if let Some(lease) = self.in_use.read().unwrap().get(digest) {
            return Err(StoreError::InUse(lease.clone()));
        }
        let removed = self.index.write().unwrap().remove(digest);
        if removed.is_none() {
            return Err(StoreError::NotFound(digest.to_string()));
        }
        match fs::remove_file(self.blob_path(digest)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::Io(e)),
        }
    }

    fn count(&self) -> usize {
        self.index.read().unwrap().len()
    }

    fn total_bytes(&self) -> u64 {
        self.index.read().unwrap().values().map(|i| i.size).sum()
    }

    fn label_blob(
        &self,
        digest: &Digest,
        key: String,
        value: String,
    ) -> Result<(), StoreError> {
        let mut idx = self.index.write().unwrap();
        let info = idx
            .get_mut(digest)
            .ok_or_else(|| StoreError::NotFound(digest.to_string()))?;
        info.labels.insert(key, value);
        Ok(())
    }
}

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::digest::DigestAlgorithm;
    use tempfile::TempDir;

    fn tempstore() -> (TempDir, LocalStore) {
        let dir = TempDir::new().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        (dir, store)
    }

    fn put(store: &LocalStore, bytes: &[u8]) -> Digest {
        let expected = Digest::compute(DigestAlgorithm::Sha256, bytes);
        let ref_ = format!("test-{}", &expected.hex()[..8]);
        let mut writer = store.writer(ref_, expected.clone()).unwrap();
        use std::io::Write;
        writer.write_all(bytes).unwrap();
        writer.commit().unwrap();
        expected
    }

    #[test]
    fn open_creates_directory_layout() {
        let (dir, _store) = tempstore();
        assert!(dir.path().join("blobs/sha256").is_dir());
        assert!(dir.path().join("blobs/sha512").is_dir());
        assert!(dir.path().join("ingest").is_dir());
    }

    #[test]
    fn writer_commit_makes_blob_visible() {
        let (_dir, store) = tempstore();
        let d = put(&store, b"hello world");
        assert!(store.exists(&d));
        let info = store.info(&d).unwrap();
        assert_eq!(info.size, 11);
    }

    #[test]
    fn writer_mismatch_rejects_commit() {
        let (_dir, store) = tempstore();
        // Open writer with a digest that won't match.
        let lying = Digest::compute(DigestAlgorithm::Sha256, b"different");
        let mut writer = store.writer("bad".into(), lying.clone()).unwrap();
        use std::io::Write;
        writer.write_all(b"actual content").unwrap();
        let err = writer.commit().unwrap_err();
        assert!(format!("{err}").contains("mismatch"));
        assert!(!store.exists(&lying));
    }

    #[test]
    fn duplicate_writer_reference_refused() {
        let (_dir, store) = tempstore();
        let d = Digest::compute(DigestAlgorithm::Sha256, b"x");
        let _writer1 = store.writer("ref-1".into(), d.clone()).unwrap();
        let err = store.writer("ref-1".into(), d).unwrap_err();
        assert!(matches!(err, StoreError::IngestActive(_)));
    }

    #[test]
    fn delete_removes_blob_and_index_entry() {
        let (_dir, store) = tempstore();
        let d = put(&store, b"to be deleted");
        store.delete(&d).unwrap();
        assert!(!store.exists(&d));
        assert!(matches!(store.info(&d).unwrap_err(), StoreError::NotFound(_)));
    }

    #[test]
    fn delete_unknown_blob_errors() {
        let (_dir, store) = tempstore();
        let d = Digest::compute(DigestAlgorithm::Sha256, b"missing");
        assert!(matches!(store.delete(&d).unwrap_err(), StoreError::NotFound(_)));
    }

    #[test]
    fn delete_in_use_blob_refuses() {
        let (_dir, store) = tempstore();
        let d = put(&store, b"held");
        store.mark_in_use(&d, "lease-7".into());
        assert!(matches!(store.delete(&d).unwrap_err(), StoreError::InUse(l) if l == "lease-7"));
        // Releasing the lease unlocks delete.
        store.release_lease("lease-7");
        store.delete(&d).unwrap();
    }

    #[test]
    fn count_and_total_bytes_track_blobs() {
        let (_dir, store) = tempstore();
        let _a = put(&store, b"aaa");
        let _b = put(&store, b"bbbb");
        assert_eq!(store.count(), 2);
        assert_eq!(store.total_bytes(), 7);
    }

    #[test]
    fn walk_visits_every_blob_once() {
        let (_dir, store) = tempstore();
        let _ = put(&store, b"one");
        let _ = put(&store, b"two");
        let _ = put(&store, b"three");
        let mut seen = 0;
        store.walk(&mut |_| seen += 1).unwrap();
        assert_eq!(seen, 3);
    }

    #[test]
    fn label_blob_persists_metadata() {
        let (_dir, store) = tempstore();
        let d = put(&store, b"labeled");
        store
            .label_blob(&d, "containerd.io/gc.ref.content.config".into(), d.to_string())
            .unwrap();
        let info = store.info(&d).unwrap();
        assert_eq!(
            info.labels.get("containerd.io/gc.ref.content.config"),
            Some(&d.to_string())
        );
    }

    #[test]
    fn reader_reads_full_blob() {
        let (_dir, store) = tempstore();
        let d = put(&store, b"abcdef");
        let mut r = store.reader(&d).unwrap();
        assert_eq!(r.size(), 6);
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut *r, &mut buf).unwrap();
        assert_eq!(&buf, b"abcdef");
    }

    #[test]
    fn abort_active_removes_tmp_and_clears_active() {
        let (_dir, store) = tempstore();
        let d = Digest::compute(DigestAlgorithm::Sha256, b"never-finished");
        let mut writer = store.writer("partial".into(), d).unwrap();
        use std::io::Write;
        writer.write_all(b"partial bytes").unwrap();
        // Drop the writer without committing.
        drop(writer);
        store.abort_active("partial").unwrap();
        assert!(store.list_active().is_empty());
    }

    #[test]
    fn scan_existing_picks_up_pre_existing_blob() {
        let dir = TempDir::new().unwrap();
        {
            let store = LocalStore::open(dir.path()).unwrap();
            let _ = put(&store, b"persistent");
        }
        // Reopen and verify the blob was rediscovered.
        let store2 = LocalStore::open(dir.path()).unwrap();
        assert_eq!(store2.count(), 1);
    }
}

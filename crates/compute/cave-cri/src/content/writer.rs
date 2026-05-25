// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Streaming ingest writer.
//!
//! Mirrors `core/content/local/writer.go`. The caller opens a writer
//! via [`super::store::LocalStore::writer`], streams bytes through
//! [`std::io::Write`], then calls [`Writer::commit`] to finalize.
//! Commit verifies the running digest matches the expected one and
//! moves the temp file into the blob layout. A dropped, never-
//! committed writer leaves its tmp file in place — callers that
//! want to clean up should follow up with
//! [`super::store::LocalStore::abort_active`].

use super::digest::{Digest, DigestAlgorithm, DigestError};
use super::store::{now_unix, ContentInfo, IngestState};
use sha2::Digest as _;
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, thiserror::Error)]
pub enum WriterError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("digest mismatch: expected {expected}, got {actual}")]
    Mismatch { expected: String, actual: String },
    #[error("digest: {0}")]
    Digest(#[from] DigestError),
}

enum Hasher {
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
}

impl Hasher {
    fn new(alg: DigestAlgorithm) -> Self {
        match alg {
            DigestAlgorithm::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            DigestAlgorithm::Sha384 => Hasher::Sha384(sha2::Sha384::new()),
            DigestAlgorithm::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Hasher::Sha256(h) => h.update(bytes),
            Hasher::Sha384(h) => h.update(bytes),
            Hasher::Sha512(h) => h.update(bytes),
        }
    }

    fn finalize(self, alg: DigestAlgorithm) -> Digest {
        let hex = match self {
            Hasher::Sha256(h) => hex_encode(&h.finalize()),
            Hasher::Sha384(h) => hex_encode(&h.finalize()),
            Hasher::Sha512(h) => hex_encode(&h.finalize()),
        };
        Digest::parse(&format!("{}:{}", alg.as_str(), hex))
            .expect("internal hex emitted by sha2 must round-trip")
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// A streaming ingest handle. Writes go to a temp file, are hashed on
/// the fly, and only become part of the store on a successful
/// [`Writer::commit`].
pub struct Writer {
    reference: String,
    expected: Digest,
    tmp_path: PathBuf,
    index: Arc<RwLock<HashMap<Digest, ContentInfo>>>,
    active: Arc<RwLock<HashMap<String, IngestState>>>,
    blobs_root: PathBuf,
    file: BufWriter<fs::File>,
    hasher: Hasher,
    bytes_written: u64,
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Writer")
            .field("reference", &self.reference)
            .field("expected", &self.expected)
            .field("tmp_path", &self.tmp_path)
            .field("bytes_written", &self.bytes_written)
            .finish()
    }
}

impl Writer {
    pub(crate) fn new(
        reference: String,
        expected: Digest,
        tmp_path: PathBuf,
        index: Arc<RwLock<HashMap<Digest, ContentInfo>>>,
        active: Arc<RwLock<HashMap<String, IngestState>>>,
        blobs_root: PathBuf,
    ) -> io::Result<Self> {
        if let Some(parent) = tmp_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        let hasher = Hasher::new(expected.algorithm());
        Ok(Self {
            reference,
            tmp_path,
            file: BufWriter::new(file),
            hasher,
            expected,
            index,
            active,
            blobs_root,
            bytes_written: 0,
        })
    }

    pub fn reference(&self) -> &str {
        &self.reference
    }

    pub fn expected(&self) -> &Digest {
        &self.expected
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Finalize the ingest. On a digest match the temp file is renamed
    /// into the blob layout and an index entry is created; on a
    /// mismatch the temp file is removed and the active entry is
    /// cleared so the reference can be retried.
    pub fn commit(mut self) -> Result<Digest, WriterError> {
        self.file.flush()?;
        let file = self
            .file
            .into_inner()
            .map_err(|e| WriterError::Io(e.into_error()))?;
        file.sync_all()?;
        drop(file);

        let actual = self.hasher.finalize(self.expected.algorithm());
        if actual != self.expected {
            let _ = fs::remove_file(&self.tmp_path);
            self.active.write().unwrap().remove(&self.reference);
            return Err(WriterError::Mismatch {
                expected: self.expected.to_string(),
                actual: actual.to_string(),
            });
        }

        let final_path = self.blobs_root.join(actual.fs_path());
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if final_path.exists() {
            // Blob already present — keep existing, just clean up the
            // temp file and refresh the index entry.
            let _ = fs::remove_file(&self.tmp_path);
        } else {
            fs::rename(&self.tmp_path, &final_path)?;
        }
        let size = fs::metadata(&final_path)?.len();
        self.index.write().unwrap().insert(
            actual.clone(),
            ContentInfo {
                digest: actual.clone(),
                size,
                labels: HashMap::new(),
                created_at_unix: now_unix(),
            },
        );
        self.active.write().unwrap().remove(&self.reference);
        Ok(actual)
    }
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.file.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes_written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::store::{ContentStore, LocalStore};
    use tempfile::TempDir;

    fn store() -> (TempDir, LocalStore) {
        let dir = TempDir::new().unwrap();
        let s = LocalStore::open(dir.path()).unwrap();
        (dir, s)
    }

    #[test]
    fn write_then_commit_persists_blob() {
        let (_d, store) = store();
        let expected = Digest::compute(DigestAlgorithm::Sha256, b"streamed bytes");
        let mut w = store.writer("r1".into(), expected.clone()).unwrap();
        w.write_all(b"streamed ").unwrap();
        w.write_all(b"bytes").unwrap();
        let got = w.commit().unwrap();
        assert_eq!(got, expected);
        assert!(store.exists(&expected));
    }

    #[test]
    fn bytes_written_counter_tracks_writes() {
        let (_d, store) = store();
        let expected = Digest::compute(DigestAlgorithm::Sha256, b"abcdef");
        let mut w = store.writer("r2".into(), expected).unwrap();
        w.write_all(b"abc").unwrap();
        assert_eq!(w.bytes_written(), 3);
        w.write_all(b"def").unwrap();
        assert_eq!(w.bytes_written(), 6);
    }

    #[test]
    fn commit_with_mismatch_removes_tmp_and_active() {
        let (_d, store) = store();
        let lying = Digest::compute(DigestAlgorithm::Sha256, b"wrong");
        let tmp_path = store.ingest_path("r3");
        let mut w = store.writer("r3".into(), lying).unwrap();
        w.write_all(b"actual bytes").unwrap();
        let err = w.commit().unwrap_err();
        assert!(format!("{err}").contains("mismatch"));
        assert!(!tmp_path.exists(), "tmp file should be cleaned up");
        assert!(store.list_active().is_empty());
    }

    #[test]
    fn writer_supports_sha512_algorithm() {
        let (_d, store) = store();
        let expected = Digest::compute(DigestAlgorithm::Sha512, b"512-bit blob");
        let mut w = store.writer("r4".into(), expected.clone()).unwrap();
        w.write_all(b"512-bit blob").unwrap();
        let got = w.commit().unwrap();
        assert_eq!(got.algorithm(), DigestAlgorithm::Sha512);
        assert_eq!(got, expected);
    }

    #[test]
    fn idempotent_commit_when_blob_already_present() {
        let (_d, store) = store();
        let expected = Digest::compute(DigestAlgorithm::Sha256, b"twice");
        // First commit.
        let mut w1 = store.writer("r5a".into(), expected.clone()).unwrap();
        w1.write_all(b"twice").unwrap();
        w1.commit().unwrap();
        // Second commit under a different reference, same digest.
        let mut w2 = store.writer("r5b".into(), expected.clone()).unwrap();
        w2.write_all(b"twice").unwrap();
        let got = w2.commit().unwrap();
        assert_eq!(got, expected);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn write_traits_unbuffered_immediate() {
        // Verify the io::Write implementation matches its returned
        // count to what gets fed into the hasher.
        let (_d, store) = store();
        let expected = Digest::compute(DigestAlgorithm::Sha256, b"x");
        let mut w = store.writer("r6".into(), expected.clone()).unwrap();
        let n = w.write(b"x").unwrap();
        assert_eq!(n, 1);
        let got = w.commit().unwrap();
        assert_eq!(got, expected);
    }
}

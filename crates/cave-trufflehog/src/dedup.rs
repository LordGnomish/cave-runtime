// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hash-based dedup keyed by (detector_type, secret, commit, file).
//! Mirrors the dedup logic inside `pkg/engine.Engine.notifyResults` —
//! upstream uses a `sync.Map` keyed by SHA-256 of those fields.

use crate::models::{DetectorType, Finding};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Default)]
pub struct Dedup {
    seen: Mutex<HashSet<String>>,
}

impl Dedup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this is the *first* time we see this fingerprint.
    /// Drop subsequent duplicates. The fingerprint mirrors the upstream
    /// ordering: detector|raw|commit|file so the same secret discovered
    /// twice in the same blob is dropped, but a secret rotated through
    /// a new commit is preserved.
    pub fn insert_finding(&self, f: &Finding) -> bool {
        let key = fingerprint(
            f.result.detector_type,
            &f.result.raw,
            f.source_metadata.commit.as_deref().unwrap_or(""),
            f.source_metadata.file.as_deref().unwrap_or(""),
        );
        let mut g = self.seen.lock().unwrap();
        g.insert(key)
    }

    pub fn insert_raw(&self, t: DetectorType, raw: &str, commit: &str, file: &str) -> bool {
        let key = fingerprint(t, raw, commit, file);
        let mut g = self.seen.lock().unwrap();
        g.insert(key)
    }

    pub fn len(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub fn fingerprint(t: DetectorType, raw: &str, commit: &str, file: &str) -> String {
    let mut h = Sha256::new();
    h.update((t as u32).to_le_bytes());
    h.update(b"|");
    h.update(raw.as_bytes());
    h.update(b"|");
    h.update(commit.as_bytes());
    h.update(b"|");
    h.update(file.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DetectionResult, SourceMetadata};

    fn mk_finding(raw: &str, commit: &str, file: &str) -> Finding {
        Finding {
            result: DetectionResult::new(DetectorType::Stripe, raw),
            chunk_source: "git".into(),
            source_metadata: SourceMetadata {
                commit: Some(commit.into()),
                file: Some(file.into()),
                ..Default::default()
            },
            redacted: format!("{}…", &raw[..raw.len().min(4)]),
        }
    }

    #[test]
    fn fingerprint_is_stable() {
        let a = fingerprint(DetectorType::Aws, "x", "c1", "/f");
        let b = fingerprint(DetectorType::Aws, "x", "c1", "/f");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn dedup_drops_duplicates() {
        let d = Dedup::new();
        let f = mk_finding("sk_live_1", "c1", "/a.go");
        assert!(d.insert_finding(&f));
        assert!(!d.insert_finding(&f));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn different_commit_is_preserved() {
        let d = Dedup::new();
        let f1 = mk_finding("sk_live_1", "c1", "/a.go");
        let f2 = mk_finding("sk_live_1", "c2", "/a.go");
        assert!(d.insert_finding(&f1));
        assert!(d.insert_finding(&f2));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn different_secret_is_preserved() {
        let d = Dedup::new();
        let f1 = mk_finding("sk_live_1", "c1", "/a.go");
        let f2 = mk_finding("sk_live_2", "c1", "/a.go");
        assert!(d.insert_finding(&f1));
        assert!(d.insert_finding(&f2));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn insert_raw_is_keyed_consistently_with_finding() {
        let d = Dedup::new();
        let f = mk_finding("sk_live_z", "c1", "/a.go");
        assert!(d.insert_finding(&f));
        assert!(!d.insert_raw(DetectorType::Stripe, "sk_live_z", "c1", "/a.go"));
    }
}

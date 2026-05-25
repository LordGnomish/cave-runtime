// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory `SignedArtifact` store — backing for the HTTP routes.
//!
//! Maps to:
//!   * pkg/cosign/remote/index.go → SignedEntity index
//!
//! Production stores route through cave-db (Postgres). This in-process
//! store is what the axum router talks to in unit tests + smoke runs.

use crate::error::{Result, SignError};
use crate::models::{ArtifactType, SignedArtifact};
use chrono::Utc;
use std::sync::{Mutex, MutexGuard};
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct SignedArtifactStore {
    inner: Mutex<Vec<SignedArtifact>>,
}

impl SignedArtifactStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Vec<SignedArtifact>>> {
        self.inner
            .lock()
            .map_err(|e| SignError::Io(format!("store lock: {}", e)))
    }

    pub fn insert(
        &self,
        artifact_digest: String,
        artifact_type: ArtifactType,
        signature: String,
        signer_identity: String,
        verified: bool,
    ) -> Result<SignedArtifact> {
        let a = SignedArtifact {
            id: Uuid::new_v4(),
            artifact_digest,
            artifact_type,
            signature,
            signer_identity,
            signed_at: Utc::now(),
            verified,
        };
        self.lock()?.push(a.clone());
        Ok(a)
    }

    pub fn all(&self) -> Result<Vec<SignedArtifact>> {
        Ok(self.lock()?.clone())
    }

    pub fn find_by_digest(&self, digest: &str) -> Result<Vec<SignedArtifact>> {
        Ok(self
            .lock()?
            .iter()
            .filter(|a| a.artifact_digest == digest)
            .cloned()
            .collect())
    }

    pub fn mark_verified(&self, id: Uuid, verified: bool) -> Result<bool> {
        let mut g = self.lock()?;
        if let Some(a) = g.iter_mut().find(|a| a.id == id) {
            a.verified = verified;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn remove(&self, id: Uuid) -> Result<bool> {
        let mut g = self.lock()?;
        let before = g.len();
        g.retain(|a| a.id != id);
        Ok(g.len() != before)
    }

    pub fn count(&self) -> Result<usize> {
        Ok(self.lock()?.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(n: usize) -> SignedArtifactStore {
        let s = SignedArtifactStore::new();
        for i in 0..n {
            s.insert(
                format!("sha256:{:064x}", i),
                ArtifactType::ContainerImage,
                "sig".into(),
                "alice@example.com".into(),
                true,
            )
            .unwrap();
        }
        s
    }

    #[test]
    fn insert_then_count() {
        let s = store_with(3);
        assert_eq!(s.count().unwrap(), 3);
    }

    #[test]
    fn find_by_digest_returns_match() {
        let s = SignedArtifactStore::new();
        let a = s
            .insert(
                "sha256:01".into(),
                ArtifactType::Binary,
                "x".into(),
                "u".into(),
                true,
            )
            .unwrap();
        let hits = s.find_by_digest("sha256:01").unwrap();
        assert_eq!(hits, vec![a]);
    }

    #[test]
    fn find_by_unknown_returns_empty() {
        let s = store_with(2);
        let hits = s.find_by_digest("sha256:ffff").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn mark_verified_toggles() {
        let s = SignedArtifactStore::new();
        let a = s
            .insert(
                "sha256:02".into(),
                ArtifactType::Blob,
                "s".into(),
                "u".into(),
                false,
            )
            .unwrap();
        assert!(s.mark_verified(a.id, true).unwrap());
        let all = s.all().unwrap();
        assert!(all.iter().any(|x| x.id == a.id && x.verified));
    }

    #[test]
    fn mark_unknown_returns_false() {
        let s = SignedArtifactStore::new();
        let unknown = Uuid::new_v4();
        assert!(!s.mark_verified(unknown, true).unwrap());
    }

    #[test]
    fn remove_drops_entry() {
        let s = SignedArtifactStore::new();
        let a = s
            .insert(
                "sha256:03".into(),
                ArtifactType::Sbom,
                "s".into(),
                "u".into(),
                false,
            )
            .unwrap();
        assert!(s.remove(a.id).unwrap());
        assert_eq!(s.count().unwrap(), 0);
    }

    #[test]
    fn remove_unknown_returns_false() {
        let s = SignedArtifactStore::new();
        assert!(!s.remove(Uuid::new_v4()).unwrap());
    }

    #[test]
    fn all_returns_clone() {
        let s = store_with(2);
        let snap = s.all().unwrap();
        s.insert(
            "sha256:04".into(),
            ArtifactType::Chart,
            "x".into(),
            "u".into(),
            true,
        )
        .unwrap();
        // The snapshot must not see the later insert.
        assert_eq!(snap.len(), 2);
        assert_eq!(s.count().unwrap(), 3);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts core::artifact (consolidates Pulp Artifact + Harbor Manifest/Blob)
//! `Artifact` + `Tag` — content-addressable unit & human pointer.
//!
//! Maps to:
//! - pulpcore `Artifact + Content` (pulp/pulpcore@0f991c2fa pulpcore/app/models/content.py)
//! - Harbor   `Manifest + Tag`     (goharbor/harbor@c80058d52 src/pkg/artifact + tag/model.go)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Canonical digest length for `sha256:<64-hex>` strings.
pub const SHA256_DIGEST_LEN: usize = 7 + 64;

/// A content-addressable unit. `digest` is `sha256:<hex>`; `media_type` is
/// the OCI media type for container artifacts and the wire MIME for Pulp
/// content (e.g. `application/x-rpm`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// `sha256:<hex>` (or `sha512:…`); always lowercase, always colon-separated.
    pub digest: String,
    /// Size in bytes; matches OCI `size` and Pulp `Artifact.size`.
    pub size: u64,
    /// Wire media type (`application/vnd.oci.image.manifest.v1+json`, etc.)
    pub media_type: String,
    /// Opaque blob reference — usually the storage path under the artifact-
    /// store root (Pulp `_artifacts/<sha>/<sha>` or Harbor blob path).
    pub blob_ref: String,
    /// All tags currently pointing at this digest.
    pub tags: Vec<Tag>,
    /// Created-at instant — needed by retention policy evaluator.
    pub created_at: DateTime<Utc>,
}

impl Artifact {
    /// Convenience constructor that sets `created_at = Utc::now()` and an
    /// empty tag set. Tests + integrations use this; the per-side ports use
    /// their own constructors.
    pub fn new(digest: impl Into<String>, size: u64, media_type: impl Into<String>) -> Self {
        let digest = digest.into();
        let media_type = media_type.into();
        Self {
            blob_ref: format!("_artifacts/{}", digest),
            digest,
            size,
            media_type,
            tags: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// True when the digest matches the canonical `sha256:<64 lower-hex>` form.
    /// Both Pulp and Harbor reject any non-canonical digest at ingest.
    pub fn has_canonical_sha256_digest(&self) -> bool {
        is_canonical_sha256(&self.digest)
    }
}

/// Human-readable pointer at an artifact digest. Mutable unless `mutable=false`
/// (Harbor immutable tag rule or Pulp `retain_repo_versions` lock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub artifact_digest: String,
    pub mutable: bool,
    /// Set when a [`crate::core::Signature`] has been attached and verified.
    pub signed_with: Option<String>,
}

impl Tag {
    pub fn new(name: impl Into<String>, digest: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            artifact_digest: digest.into(),
            mutable: true,
            signed_with: None,
        }
    }

    /// Immutable variant — used by Harbor projects with immutable-tag rules
    /// and by Pulp's retention-locked snapshots.
    pub fn immutable(name: impl Into<String>, digest: impl Into<String>) -> Self {
        let mut t = Self::new(name, digest);
        t.mutable = false;
        t
    }
}

/// Validate that `s` matches the `sha256:<64 lower-hex>` canonical form
/// (Distribution Spec v1.1 §3.1 — strict).
pub fn is_canonical_sha256(s: &str) -> bool {
    if s.len() != SHA256_DIGEST_LEN {
        return false;
    }
    if !s.starts_with("sha256:") {
        return false;
    }
    s[7..].chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_artifact_defaults_empty_tags_and_now() {
        let a = Artifact::new("sha256:00112233", 42, "application/octet-stream");
        assert!(a.tags.is_empty());
        assert_eq!(a.size, 42);
        assert_eq!(a.media_type, "application/octet-stream");
        assert_eq!(a.blob_ref, "_artifacts/sha256:00112233");
    }

    #[test]
    fn canonical_sha256_check_accepts_64_hex() {
        let good = format!("sha256:{}", "a".repeat(64));
        assert!(is_canonical_sha256(&good));
        let art = Artifact::new(&good, 1, "x");
        assert!(art.has_canonical_sha256_digest());
    }

    #[test]
    fn canonical_sha256_check_rejects_wrong_alg() {
        let bad = format!("sha512:{}", "a".repeat(64));
        assert!(!is_canonical_sha256(&bad));
    }

    #[test]
    fn canonical_sha256_check_rejects_wrong_length() {
        assert!(!is_canonical_sha256("sha256:abc"));
        let bad = format!("sha256:{}", "a".repeat(63));
        assert!(!is_canonical_sha256(&bad));
    }

    #[test]
    fn canonical_sha256_check_rejects_uppercase() {
        let bad = format!("sha256:{}", "A".repeat(64));
        assert!(!is_canonical_sha256(&bad));
    }

    #[test]
    fn tag_immutable_constructor() {
        let t = Tag::immutable("v1.0.0", "sha256:abc");
        assert!(!t.mutable);
        let t2 = Tag::new("latest", "sha256:abc");
        assert!(t2.mutable);
    }

    #[test]
    fn artifact_serde_round_trip() {
        let mut a = Artifact::new("sha256:dead", 10, "application/json");
        a.tags.push(Tag::new("v1", "sha256:dead"));
        let json = serde_json::to_string(&a).unwrap();
        let back: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}

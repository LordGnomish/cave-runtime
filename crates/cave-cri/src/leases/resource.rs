// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Typed reference held by a lease.

use crate::content::digest::Digest;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Content,
    Snapshot,
    Ingest,
}

impl ResourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Content => "content",
            ResourceKind::Snapshot => "snapshot",
            ResourceKind::Ingest => "ingest",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Resource {
    pub kind: ResourceKind,
    /// String identifier — digest wire-form for content, snapshot id
    /// for snapshots, ingest reference for in-progress writers.
    pub id: String,
}

impl Resource {
    pub fn content(digest: &Digest) -> Self {
        Self {
            kind: ResourceKind::Content,
            id: digest.to_string(),
        }
    }

    pub fn snapshot(id: impl Into<String>) -> Self {
        Self {
            kind: ResourceKind::Snapshot,
            id: id.into(),
        }
    }

    pub fn ingest(reference: impl Into<String>) -> Self {
        Self {
            kind: ResourceKind::Ingest,
            id: reference.into(),
        }
    }

    /// If this resource refers to a content blob, return the parsed
    /// digest. Returns `None` for non-content resources.
    pub fn content_digest(&self) -> Option<Digest> {
        if self.kind != ResourceKind::Content {
            return None;
        }
        Digest::parse(&self.id).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::digest::DigestAlgorithm;

    #[test]
    fn content_resource_round_trips_to_digest() {
        let d = Digest::compute(DigestAlgorithm::Sha256, b"abc");
        let r = Resource::content(&d);
        assert_eq!(r.kind, ResourceKind::Content);
        assert_eq!(r.content_digest().unwrap(), d);
    }

    #[test]
    fn snapshot_resource_has_no_digest() {
        let r = Resource::snapshot("snap-7");
        assert_eq!(r.kind, ResourceKind::Snapshot);
        assert!(r.content_digest().is_none());
    }

    #[test]
    fn ingest_resource_carries_reference() {
        let r = Resource::ingest("pull-session-9");
        assert_eq!(r.kind, ResourceKind::Ingest);
        assert_eq!(r.id, "pull-session-9");
    }

    #[test]
    fn kind_as_str_stable() {
        assert_eq!(ResourceKind::Content.as_str(), "content");
        assert_eq!(ResourceKind::Snapshot.as_str(), "snapshot");
        assert_eq!(ResourceKind::Ingest.as_str(), "ingest");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts core::repository (consolidates upstream Repository / Project / Repo concepts)
//! Repository + Distribution traits.
//!
//! Maps to:
//! - pulpcore `Repository` (pulp/pulpcore@0f991c2fa pulpcore/app/models/repository.py)
//! - Harbor   `Project + Repository` (goharbor/harbor@c80058d52 src/pkg/project + repository/model.go)
//!
//! The trait is intentionally read-side only — mutation is upstream-faithful
//! per side and uses the upstream-shaped types (Pulp's Repository::add_content
//! through RepositoryVersion vs Harbor's manifest PUT through OCI v1.1).

use super::Artifact;
use serde::{Deserialize, Serialize};
use std::fmt;

/// What kind of content a repository holds. Maps to Pulp's
/// `pulp_<plugin>` namespace and Harbor's manifest media-type family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryKind {
    Container,
    Rpm,
    Deb,
    Python,
    Maven,
    Ansible,
    Chef,
    Helm,
    File,
    Ostree,
    Raw,
}

impl RepositoryKind {
    /// Wire name as used by both Pulp's `pulp_<plugin>` and Harbor's
    /// artifact-type tag.
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Rpm => "rpm",
            Self::Deb => "deb",
            Self::Python => "python",
            Self::Maven => "maven",
            Self::Ansible => "ansible",
            Self::Chef => "chef",
            Self::Helm => "helm",
            Self::File => "file",
            Self::Ostree => "ostree",
            Self::Raw => "raw",
        }
    }

    pub fn try_from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "container" => Self::Container,
            "rpm" => Self::Rpm,
            "deb" => Self::Deb,
            "python" => Self::Python,
            "maven" => Self::Maven,
            "ansible" => Self::Ansible,
            "chef" => Self::Chef,
            "helm" => Self::Helm,
            "file" => Self::File,
            "ostree" => Self::Ostree,
            "raw" => Self::Raw,
            _ => return None,
        })
    }
}

impl fmt::Display for RepositoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

/// Common read-side surface that both Pulp and Harbor sides implement so
/// that cross-cutting code (retention, scan, replication, portal) does
/// not need to know which side answered the lookup.
pub trait Repository: Send + Sync {
    /// Stable unique id (Pulp pulp_id, Harbor `<project>/<repo>` key, etc.)
    fn id(&self) -> &str;
    /// Human-readable name displayed in UI.
    fn name(&self) -> &str;
    /// Content kind.
    fn kind(&self) -> RepositoryKind;
    /// Lookup an artifact by digest (`sha256:<hex>`). Returns `None` if absent.
    fn lookup_artifact(&self, digest: &str) -> Option<Artifact>;
    /// Enumerate all artifacts (used by retention evaluator + replication).
    fn list_artifacts(&self) -> Vec<Artifact>;
    /// Total number of artifacts — fast-path used by dashboard panels.
    fn count(&self) -> usize {
        self.list_artifacts().len()
    }
}

/// How the repository is served to outside clients (a Pulp Distribution or
/// a Harbor public/proxy/replication base-path).
///
/// `base_path` is the URL prefix below which the repo's content is served
/// (`/pulp/foo/`, `/v2/library/nginx/`, etc.) and `content_type` is the
/// HTTP `Content-Type` of the repo's index page.
pub trait Distribution: Send + Sync {
    fn base_path(&self) -> &str;
    fn content_type(&self) -> &str {
        "application/json"
    }
    /// Build a `serve` response for `path` underneath `base_path`. Returns
    /// the artifact digest the caller should fetch (or `None` if no match).
    fn serve(&self, path: &str) -> Option<String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_wire_round_trip_all_variants() {
        let all = [
            RepositoryKind::Container,
            RepositoryKind::Rpm,
            RepositoryKind::Deb,
            RepositoryKind::Python,
            RepositoryKind::Maven,
            RepositoryKind::Ansible,
            RepositoryKind::Chef,
            RepositoryKind::Helm,
            RepositoryKind::File,
            RepositoryKind::Ostree,
            RepositoryKind::Raw,
        ];
        for k in all {
            let w = k.as_wire();
            let back = RepositoryKind::try_from_wire(w).expect("wire round trips");
            assert_eq!(k, back, "round-trip mismatch for {w}");
        }
    }

    #[test]
    fn kind_try_from_unknown_returns_none() {
        assert!(RepositoryKind::try_from_wire("snowflake").is_none());
        assert!(RepositoryKind::try_from_wire("").is_none());
    }

    #[test]
    fn kind_display_matches_wire() {
        assert_eq!(format!("{}", RepositoryKind::Container), "container");
        assert_eq!(format!("{}", RepositoryKind::Rpm), "rpm");
    }

    // Compile-time trait-object check — exercises both traits as `dyn`.
    #[allow(dead_code)]
    fn _accepts_trait_object(r: &dyn Repository, d: &dyn Distribution) -> (String, String) {
        (r.id().to_string(), d.base_path().to_string())
    }
}

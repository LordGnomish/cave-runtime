// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_ansible — Ansible collection and role content plugin.
//!
//! Mirrors `pulp_ansible`:
//! - Filename parser: `{namespace}-{name}-{version}.tar.gz` for collections
//!   (canonical), `{user}.{role}-{version}.tar.gz` for legacy roles.
//! - Galaxy v3 API JSON renderers:
//!   - per-collection index (`/collections/index/{ns}/{name}/`)
//!   - per-collection-version (`/collections/index/{ns}/{name}/versions/{v}/`)
//!   - listing (`/collections/`).
//! - Semver-aware version sorting (PEP 440-lite: split on `.`, numeric
//!   compare per segment, pre-release tags `-rc`, `-alpha`, `-beta` sort
//!   before plain).

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;

pub struct AnsiblePlugin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnsibleArtifact {
    Collection {
        namespace: String,
        name: String,
        version: String,
    },
    Role {
        user: String,
        name: String,
        version: String,
    },
}

impl AnsibleArtifact {
    pub fn version(&self) -> &str {
        match self {
            Self::Collection { version, .. } | Self::Role { version, .. } => version,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Collection { .. } => "collection",
            Self::Role { .. } => "role",
        }
    }
}

/// Parse `{namespace}-{name}-{version}.tar.gz` (collection) or
/// `{user}.{role}-{version}.tar.gz` (legacy role).
///
/// Heuristic: legacy roles have exactly two `-`-separated segments and the
/// first segment contains a `.` (the `user.role` namespace dot). Anything
/// else with at least three segments is treated as a collection — the
/// remainder after `{ns}-{name}-` is the full version, even when the
/// version itself contains `-` (e.g. `2.0.0-rc1`).
pub fn parse_ansible_filename(filename: &str) -> Option<AnsibleArtifact> {
    let stem = filename.strip_suffix(".tar.gz")?;
    let segments: Vec<&str> = stem.split('-').collect();
    if segments.len() < 2 {
        return None;
    }
    if segments.len() == 2 && segments[0].contains('.') {
        let (user, role) = segments[0].split_once('.')?;
        return Some(AnsibleArtifact::Role {
            user: user.to_string(),
            name: role.to_string(),
            version: segments[1].to_string(),
        });
    }
    if segments.len() < 3 {
        return None;
    }
    let version = segments[2..].join("-");
    Some(AnsibleArtifact::Collection {
        namespace: segments[0].to_string(),
        name: segments[1].to_string(),
        version,
    })
}

/// Semver-aware comparator. Splits on `.`; segments parsed as integers
/// compare numerically; trailing `-rcN`, `-alphaN`, `-betaN` sort before
/// the bare version (matches Galaxy's `packaging.version.Version`).
pub fn cmp_semver(a: &str, b: &str) -> Ordering {
    let (a_core, a_pre) = split_prerelease(a);
    let (b_core, b_pre) = split_prerelease(b);

    let a_segments: Vec<u64> = a_core.split('.').filter_map(|s| s.parse().ok()).collect();
    let b_segments: Vec<u64> = b_core.split('.').filter_map(|s| s.parse().ok()).collect();

    for (lhs, rhs) in a_segments.iter().zip(b_segments.iter()) {
        match lhs.cmp(rhs) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    match a_segments.len().cmp(&b_segments.len()) {
        Ordering::Equal => {}
        other => return other,
    }

    match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

fn split_prerelease(s: &str) -> (&str, Option<&str>) {
    for marker in &["-rc", "-alpha", "-beta", "-pre", "-dev"] {
        if let Some(idx) = s.find(marker) {
            return (&s[..idx], Some(&s[idx..]));
        }
    }
    (s, None)
}

impl ArtifactsPlugin for AnsiblePlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Ansible
    }

    fn name(&self) -> &str {
        "pulp_ansible"
    }

    fn content_types(&self) -> Vec<&str> {
        vec![
            "ansible.collection_version",
            "ansible.role",
            "ansible.collection_signature",
        ]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let parsed = parse_ansible_filename(filename).ok_or_else(|| {
            ArtifactsError::InvalidRequest(format!("not an ansible artifact: {filename}"))
        })?;
        let sha256 = hex::encode(Sha256::digest(data));

        let metadata = match &parsed {
            AnsibleArtifact::Collection { namespace, name, version } => serde_json::json!({
                "namespace": namespace,
                "name": name,
                "version": version,
                "filename": filename,
                "kind": "collection",
                "sha256": sha256,
            }),
            AnsibleArtifact::Role { user, name, version } => serde_json::json!({
                "namespace": user,
                "name": name,
                "version": version,
                "filename": filename,
                "kind": "role",
                "sha256": sha256,
            }),
        };

        let mut unit = ContentUnit::new(PluginType::Ansible, metadata);
        unit.relative_path = Some(relative_path.to_string());
        unit.sha256 = Some(sha256);
        unit.size = Some(data.len() as u64);
        Ok(unit)
    }

    fn generate_metadata(
        &self,
        _repo_version: &RepositoryVersion,
        units: &[ContentUnit],
    ) -> serde_json::Value {
        let mut by_collection: BTreeMap<(String, String), Vec<&ContentUnit>> = BTreeMap::new();
        for u in units {
            if u.metadata.get("kind").and_then(|v| v.as_str()) != Some("collection") {
                continue;
            }
            let ns = u.metadata["namespace"].as_str().unwrap_or("").to_string();
            let name = u.metadata["name"].as_str().unwrap_or("").to_string();
            by_collection.entry((ns, name)).or_default().push(u);
        }

        for v in by_collection.values_mut() {
            v.sort_by(|a, b| {
                let av = a.metadata["version"].as_str().unwrap_or("");
                let bv = b.metadata["version"].as_str().unwrap_or("");
                cmp_semver(bv, av) // descending: highest first
            });
        }

        let collections: Vec<serde_json::Value> = by_collection
            .iter()
            .map(|((ns, name), versions)| {
                let highest = versions
                    .first()
                    .map(|u| u.metadata["version"].as_str().unwrap_or(""))
                    .unwrap_or("");
                serde_json::json!({
                    "namespace": ns,
                    "name": name,
                    "highest_version": {
                        "version": highest,
                        "href": format!("/api/v3/plugin/ansible/content/default/collections/index/{ns}/{name}/versions/{highest}/"),
                    },
                    "versions_url": format!("/api/v3/plugin/ansible/content/default/collections/index/{ns}/{name}/versions/"),
                })
            })
            .collect();

        let mut version_indexes: serde_json::Map<String, serde_json::Value> =
            serde_json::Map::new();
        for ((ns, name), versions) in &by_collection {
            let arr: Vec<serde_json::Value> = versions
                .iter()
                .map(|u| {
                    let v = u.metadata["version"].as_str().unwrap_or("");
                    serde_json::json!({
                        "version": v,
                        "href": format!("/api/v3/plugin/ansible/content/default/collections/index/{ns}/{name}/versions/{v}/"),
                        "download_url": format!("/pulp/content/ansible/collections/{ns}-{name}-{v}.tar.gz"),
                        "sha256": u.sha256,
                    })
                })
                .collect();
            version_indexes.insert(
                format!("{ns}/{name}"),
                serde_json::json!({ "meta": { "count": arr.len() }, "data": arr }),
            );
        }

        serde_json::json!({
            "collections": collections,
            "count": collections.len(),
            "version_indexes": serde_json::Value::Object(version_indexes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_collection_filename() {
        let a = parse_ansible_filename("community-general-7.3.0.tar.gz").unwrap();
        match a {
            AnsibleArtifact::Collection { namespace, name, version } => {
                assert_eq!(namespace, "community");
                assert_eq!(name, "general");
                assert_eq!(version, "7.3.0");
            }
            _ => panic!("expected collection"),
        }
    }

    #[test]
    fn parse_legacy_role_filename() {
        let a = parse_ansible_filename("geerlingguy.docker-1.5.2.tar.gz").unwrap();
        match a {
            AnsibleArtifact::Role { user, name, version } => {
                assert_eq!(user, "geerlingguy");
                assert_eq!(name, "docker");
                assert_eq!(version, "1.5.2");
            }
            _ => panic!("expected role"),
        }
    }

    #[test]
    fn parse_rejects_non_tarball() {
        assert!(parse_ansible_filename("community-general-7.3.0.zip").is_none());
    }

    #[test]
    fn semver_compare_basic() {
        assert_eq!(cmp_semver("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(cmp_semver("1.0.0", "1.0.1"), Ordering::Less);
        assert_eq!(cmp_semver("1.2.3", "1.2.4"), Ordering::Less);
        assert_eq!(cmp_semver("2.0.0", "1.99.99"), Ordering::Greater);
    }

    #[test]
    fn semver_prerelease_sorts_before_release() {
        assert_eq!(cmp_semver("1.0.0-rc1", "1.0.0"), Ordering::Less);
        assert_eq!(cmp_semver("1.0.0-alpha", "1.0.0-beta"), Ordering::Less);
        assert_eq!(cmp_semver("1.0.0-beta", "1.0.0-rc1"), Ordering::Less);
    }

    #[test]
    fn semver_handles_different_length_versions() {
        assert_eq!(cmp_semver("1.0", "1.0.0"), Ordering::Less);
        assert_eq!(cmp_semver("1.0.0", "1.0"), Ordering::Greater);
    }

    #[test]
    fn galaxy_index_sorts_versions_descending() {
        let plugin = AnsiblePlugin;
        let v1 = plugin
            .parse_content(b"x", "community-general-1.0.0.tar.gz")
            .unwrap();
        let v2 = plugin
            .parse_content(b"x", "community-general-2.0.0.tar.gz")
            .unwrap();
        let v3 = plugin
            .parse_content(b"x", "community-general-2.0.0-rc1.tar.gz")
            .unwrap();
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/ansible/ansible/x/", 1);
        let meta = plugin.generate_metadata(&ver, &[v1, v2, v3]);
        let idx = meta["version_indexes"]["community/general"]["data"]
            .as_array()
            .unwrap();
        assert_eq!(idx[0]["version"], "2.0.0");
        assert_eq!(idx[1]["version"], "2.0.0-rc1");
        assert_eq!(idx[2]["version"], "1.0.0");
    }

    #[test]
    fn collection_listing_picks_highest_version() {
        let plugin = AnsiblePlugin;
        let units = vec![
            plugin
                .parse_content(b"x", "community-general-1.0.0.tar.gz")
                .unwrap(),
            plugin
                .parse_content(b"x", "community-general-2.0.0.tar.gz")
                .unwrap(),
        ];
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/ansible/ansible/x/", 1);
        let meta = plugin.generate_metadata(&ver, &units);
        let coll = &meta["collections"][0];
        assert_eq!(coll["namespace"], "community");
        assert_eq!(coll["name"], "general");
        assert_eq!(coll["highest_version"]["version"], "2.0.0");
        assert!(coll["highest_version"]["href"]
            .as_str()
            .unwrap()
            .contains("/versions/2.0.0/"));
    }

    #[test]
    fn parse_content_marks_collection_kind() {
        let plugin = AnsiblePlugin;
        let u = plugin
            .parse_content(b"x", "ansible-posix-1.5.0.tar.gz")
            .unwrap();
        assert_eq!(u.metadata["kind"], "collection");
        assert_eq!(u.metadata["namespace"], "ansible");
        assert_eq!(u.metadata["name"], "posix");
    }

    #[test]
    fn parse_content_marks_role_kind() {
        let plugin = AnsiblePlugin;
        let u = plugin
            .parse_content(b"x", "geerlingguy.docker-1.5.2.tar.gz")
            .unwrap();
        assert_eq!(u.metadata["kind"], "role");
        assert_eq!(u.metadata["namespace"], "geerlingguy");
    }

    #[test]
    fn content_types_match_pulp_ansible_models() {
        let plugin = AnsiblePlugin;
        let kinds = plugin.content_types();
        for required in [
            "ansible.collection_version",
            "ansible.role",
            "ansible.collection_signature",
        ] {
            assert!(kinds.contains(&required), "missing kind {required}");
        }
    }
}

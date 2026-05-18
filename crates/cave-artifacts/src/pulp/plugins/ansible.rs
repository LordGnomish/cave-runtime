// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_ansible — Ansible collection and role content plugin.
//!
//! Implements:
//! - MANIFEST.json reader (`parse_collection_manifest`).
//! - FILES.json reader (`parse_collection_files`).
//! - Galaxy v3 API response body composer (`galaxy_v3_response`).
//! - meta/main.yml reader for legacy roles (`parse_role_meta`).
//!
//! Upstream parity: pulp/pulp_ansible `pulp_ansible/app/models.py` +
//! Ansible Galaxy v3 API spec.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub struct AnsiblePlugin;

// ── MANIFEST.json ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CollectionInfo {
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub readme: Option<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub license: Vec<String>,
    pub license_file: Option<String>,
    pub dependencies: HashMap<String, String>,
    pub repository: Option<String>,
    pub documentation: Option<String>,
    pub homepage: Option<String>,
    pub issues: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FileManifestFile {
    pub name: String,
    pub ftype: String,
    #[serde(default)]
    pub chksum_type: Option<String>,
    #[serde(default)]
    pub chksum_sha256: Option<String>,
    #[serde(default)]
    pub format: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CollectionManifest {
    pub collection_info: CollectionInfo,
    pub file_manifest_file: Option<FileManifestFile>,
    #[serde(default = "default_format")]
    pub format: u32,
}

fn default_format() -> u32 {
    1
}

pub fn parse_collection_manifest(raw: &str) -> Result<CollectionManifest, ArtifactsError> {
    let m: CollectionManifest = serde_json::from_str(raw)
        .map_err(|e| ArtifactsError::InvalidRequest(format!("MANIFEST.json: {e}")))?;
    if m.collection_info.namespace.is_empty() {
        return Err(ArtifactsError::InvalidRequest(
            "MANIFEST.json: missing collection_info.namespace".into(),
        ));
    }
    if m.collection_info.name.is_empty() {
        return Err(ArtifactsError::InvalidRequest(
            "MANIFEST.json: missing collection_info.name".into(),
        ));
    }
    Ok(m)
}

// ── FILES.json ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CollectionFileEntry {
    pub name: String,
    pub ftype: String,
    #[serde(default)]
    pub chksum_type: Option<String>,
    #[serde(default)]
    pub chksum_sha256: Option<String>,
    #[serde(default)]
    pub format: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CollectionFiles {
    pub files: Vec<CollectionFileEntry>,
    #[serde(default = "default_format")]
    pub format: u32,
}

pub fn parse_collection_files(raw: &str) -> Result<CollectionFiles, ArtifactsError> {
    serde_json::from_str(raw).map_err(|e| ArtifactsError::InvalidRequest(format!("FILES.json: {e}")))
}

// ── Galaxy v3 API response ──────────────────────────────────────────────────

/// Compose a Galaxy v3 single-version response body.
pub fn galaxy_v3_response(m: &CollectionManifest, base_url: &str) -> String {
    let ci = &m.collection_info;
    let filename = format!("{}-{}-{}.tar.gz", ci.namespace, ci.name, ci.version);
    let download_url = format!("{}/download/{filename}", base_url.trim_end_matches('/'));
    serde_json::json!({
        "version": ci.version,
        "namespace": { "name": ci.namespace },
        "collection": { "name": ci.name },
        "metadata": {
            "tags": ci.tags,
            "authors": ci.authors,
            "description": ci.description,
            "license": ci.license,
            "dependencies": ci.dependencies,
            "homepage": ci.homepage,
            "repository": ci.repository,
            "documentation": ci.documentation,
            "issues": ci.issues,
        },
        "download_url": download_url,
        "artifact": {
            "filename": filename,
            "sha256": m.file_manifest_file.as_ref().and_then(|f| f.chksum_sha256.clone()),
        }
    })
    .to_string()
}

// ── role meta/main.yml ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RolePlatform {
    pub name: String,
    #[serde(deserialize_with = "deserialize_versions")]
    pub versions: Vec<String>,
}

// Versions can be either strings or numbers ("22.04" vs 22.04 in YAML).
fn deserialize_versions<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Vec<serde_yaml::Value> = Vec::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|v| match v {
            serde_yaml::Value::String(s) => s,
            serde_yaml::Value::Number(n) => n.to_string(),
            other => format!("{:?}", other),
        })
        .collect())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct GalaxyInfo {
    pub author: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub min_ansible_version: Option<String>,
    pub platforms: Vec<RolePlatform>,
    pub galaxy_tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RoleMeta {
    pub galaxy_info: GalaxyInfo,
    pub dependencies: Vec<serde_yaml::Value>,
}

pub fn parse_role_meta(raw: &str) -> Result<RoleMeta, ArtifactsError> {
    serde_yaml::from_str(raw).map_err(|e| ArtifactsError::InvalidRequest(format!("role meta: {e}")))
}

// ── Plugin trait ────────────────────────────────────────────────────────────

impl ArtifactsPlugin for AnsiblePlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Ansible
    }

    fn name(&self) -> &str {
        "pulp_ansible"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["ansible.collection_version", "ansible.role"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let (mut namespace, mut name, mut version) = parse_collection_filename(filename);
        let mut tags: Vec<String> = Vec::new();
        let mut description: Option<String> = None;
        let mut license: Vec<String> = Vec::new();

        // Try to open a collection tarball and pull MANIFEST.json out.
        if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            if let Some(manifest_body) = extract_file_from_targz(data, "MANIFEST.json") {
                if let Ok(m) = parse_collection_manifest(&manifest_body) {
                    namespace = m.collection_info.namespace.clone();
                    name = m.collection_info.name.clone();
                    version = m.collection_info.version.clone();
                    tags = m.collection_info.tags;
                    description = m.collection_info.description;
                    license = m.collection_info.license;
                }
            }
        }

        let sha256 = hex::encode(Sha256::digest(data));
        let mut md = serde_json::json!({
            "namespace": namespace,
            "name": name,
            "version": version,
            "filename": filename,
            "sha256": sha256,
            "tags": tags,
            "license": license,
        });
        if let Some(d) = description {
            md["description"] = serde_json::Value::String(d);
        }
        let mut unit = ContentUnit::new(PluginType::Ansible, md);
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
        // Galaxy v3 results array — one entry per collection version.
        let collections: Vec<serde_json::Value> = units
            .iter()
            .map(|u| {
                let ns = u.metadata.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                let nm = u.metadata.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let vr = u.metadata.get("version").and_then(|v| v.as_str()).unwrap_or("");
                serde_json::json!({
                    "namespace": ns,
                    "name": nm,
                    "version": vr,
                    "download_url": format!("../../artifacts/{ns}-{nm}-{vr}.tar.gz"),
                })
            })
            .collect();
        serde_json::json!({ "results": collections, "count": collections.len() })
    }
}

fn parse_collection_filename(filename: &str) -> (String, String, String) {
    let stem = filename
        .strip_suffix(".tar.gz")
        .or_else(|| filename.strip_suffix(".tgz"))
        .unwrap_or(filename);
    let parts: Vec<&str> = stem.splitn(3, '-').collect();
    (
        parts.first().copied().unwrap_or("unknown").to_string(),
        parts.get(1).copied().unwrap_or("unknown").to_string(),
        parts.get(2).copied().unwrap_or("0.0.0").to_string(),
    )
}

fn extract_file_from_targz(tgz: &[u8], target: &str) -> Option<String> {
    use std::io::Read;
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(tgz).read_to_end(&mut decoded).ok()?;
    let mut a = tar::Archive::new(&decoded[..]);
    for entry in a.entries().ok()? {
        let mut e = entry.ok()?;
        let path = e.path().ok()?.into_owned();
        let p = path.to_string_lossy();
        if p == target || p.ends_with(&format!("/{target}")) {
            let mut s = String::new();
            e.read_to_string(&mut s).ok()?;
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_split_fallback() {
        let (ns, name, ver) = parse_collection_filename("community-general-7.3.0.tar.gz");
        assert_eq!(ns, "community");
        assert_eq!(name, "general");
        assert_eq!(ver, "7.3.0");
    }
}

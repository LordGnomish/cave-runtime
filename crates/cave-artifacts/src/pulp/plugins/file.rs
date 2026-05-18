// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_file — generic file content plugin.
//!
//! Mirrors `pulp_file`:
//! - PULP_MANIFEST CSV format (`{relative_path},{sha256_digest},{size_bytes}`).
//! - Lossless parse + render round-trip.
//! - `parse_manifest()` for ingesting an upstream PULP_MANIFEST during sync.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct FilePlugin;

/// One row in a PULP_MANIFEST CSV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub relative_path: String,
    pub sha256: String,
    pub size: u64,
}

impl FilePlugin {
    /// Convenience builder used by tests and sync.
    pub fn build_entry(relative_path: &str, data: &[u8]) -> ManifestEntry {
        ManifestEntry {
            relative_path: relative_path.to_string(),
            sha256: hex::encode(Sha256::digest(data)),
            size: data.len() as u64,
        }
    }
}

impl ArtifactsPlugin for FilePlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::File
    }

    fn name(&self) -> &str {
        "pulp_file"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["file.file"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        if relative_path.is_empty() {
            return Err(ArtifactsError::InvalidRequest(
                "file content requires a relative_path".into(),
            ));
        }
        let sha256 = hex::encode(Sha256::digest(data));
        let mut unit = ContentUnit::new(
            PluginType::File,
            serde_json::json!({
                "relative_path": relative_path,
                "digest": sha256,
            }),
        );
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
        let entries: Vec<ManifestEntry> = units
            .iter()
            .filter_map(|u| {
                Some(ManifestEntry {
                    relative_path: u.relative_path.clone()?,
                    sha256: u.sha256.clone()?,
                    size: u.size?,
                })
            })
            .collect();
        serde_json::json!({
            "manifest_csv": render_manifest(&entries),
            "manifest": entries.iter().map(|e| serde_json::json!({
                "relative_path": e.relative_path,
                "sha256": e.sha256,
                "size": e.size,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Render a sorted PULP_MANIFEST CSV. Sorting matches upstream's stable
/// output (sort by relative_path) so the published file is deterministic.
pub fn render_manifest(entries: &[ManifestEntry]) -> String {
    let mut sorted: Vec<&ManifestEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut out = String::new();
    for e in sorted {
        out.push_str(&format!("{},{},{}\n", e.relative_path, e.sha256, e.size));
    }
    out
}

/// Parse a PULP_MANIFEST CSV body. Blank lines are skipped; malformed rows
/// return an error (matching pulp_file's strict-validation behaviour).
pub fn parse_manifest(body: &str) -> Result<Vec<ManifestEntry>, ArtifactsError> {
    let mut out = Vec::new();
    for (lineno, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, ',').collect();
        if parts.len() != 3 {
            return Err(ArtifactsError::InvalidRequest(format!(
                "manifest line {}: expected `path,sha256,size`, got {parts:?}",
                lineno + 1
            )));
        }
        let size: u64 = parts[2].parse().map_err(|_| {
            ArtifactsError::InvalidRequest(format!(
                "manifest line {}: size {} is not an integer",
                lineno + 1,
                parts[2]
            ))
        })?;
        if parts[1].len() != 64 || !parts[1].chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ArtifactsError::InvalidRequest(format!(
                "manifest line {}: invalid sha256 digest {}",
                lineno + 1,
                parts[1]
            )));
        }
        out.push(ManifestEntry {
            relative_path: parts[0].to_string(),
            sha256: parts[1].to_string(),
            size,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file_content_records_sha256_and_size() {
        let plugin = FilePlugin;
        let unit = plugin.parse_content(b"hello world", "docs/readme.txt").unwrap();
        assert_eq!(unit.plugin_type, PluginType::File);
        assert_eq!(unit.relative_path.as_deref(), Some("docs/readme.txt"));
        assert_eq!(unit.size, Some(11));
        let sha = unit.sha256.unwrap();
        assert_eq!(sha.len(), 64);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parse_file_rejects_empty_relative_path() {
        let plugin = FilePlugin;
        let err = plugin.parse_content(b"x", "").unwrap_err();
        assert!(matches!(err, ArtifactsError::InvalidRequest(_)));
    }

    #[test]
    fn render_manifest_csv_is_sorted_and_canonical() {
        let entries = vec![
            FilePlugin::build_entry("zzz.txt", b"z"),
            FilePlugin::build_entry("aaa.txt", b"a"),
        ];
        let csv = render_manifest(&entries);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("aaa.txt,"));
        assert!(lines[1].starts_with("zzz.txt,"));
        let parts: Vec<&str> = lines[0].split(',').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2], "1");
    }

    #[test]
    fn parse_manifest_round_trips_render_manifest() {
        let entries = vec![
            FilePlugin::build_entry("a.txt", b"hello"),
            FilePlugin::build_entry("b/c.txt", b"world"),
        ];
        let csv = render_manifest(&entries);
        let parsed = parse_manifest(&csv).unwrap();
        let mut sorted_in = entries.clone();
        sorted_in.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        assert_eq!(parsed, sorted_in);
    }

    #[test]
    fn parse_manifest_skips_blank_lines() {
        let csv = "\n\na.txt,e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855,0\n\n";
        let parsed = parse_manifest(csv).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].relative_path, "a.txt");
    }

    #[test]
    fn parse_manifest_rejects_short_digest() {
        let csv = "a.txt,abc,0\n";
        assert!(parse_manifest(csv).is_err());
    }

    #[test]
    fn parse_manifest_rejects_non_integer_size() {
        let csv =
            "a.txt,e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855,notnum\n";
        assert!(parse_manifest(csv).is_err());
    }

    #[test]
    fn parse_manifest_rejects_malformed_row() {
        let csv = "missing-fields\n";
        assert!(parse_manifest(csv).is_err());
    }

    #[test]
    fn generate_metadata_emits_csv_and_struct_view() {
        let plugin = FilePlugin;
        let units = vec![
            plugin.parse_content(b"a", "a.txt").unwrap(),
            plugin.parse_content(b"bb", "b.txt").unwrap(),
        ];
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/file/file/x/", 1);
        let meta = plugin.generate_metadata(&ver, &units);
        let csv = meta["manifest_csv"].as_str().unwrap();
        assert_eq!(csv.lines().count(), 2);
        assert!(csv.contains(",1\n") || csv.contains(",2\n"));
        let manifest = meta["manifest"].as_array().unwrap();
        assert_eq!(manifest.len(), 2);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_file — generic file content plugin with PULP_MANIFEST CSV parser.
//!
//! Upstream: pulp/pulp_file `pulp_file/app/models.py` + `pulp_file/manifest.py`.
//! The PULP_MANIFEST format is a 3-column CSV (no header) of
//! `relative_path,sha256_digest,size_bytes` — one line per artifact.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct FilePlugin;

/// One row of a PULP_MANIFEST file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulpManifestEntry {
    pub relative_path: String,
    pub sha256: String,
    pub size: u64,
}

/// Parse a PULP_MANIFEST CSV body into entries.
///
/// Rules (upstream `pulp_file.manifest.Manifest.read`):
/// - Three comma-separated fields per line, no quoting (Pulp itself
///   rejects commas in relative paths upstream with `validate=True`).
/// - Blank lines and `#`-prefixed comment lines are skipped.
/// - Trailing newline is optional; CRLF tolerated.
/// - `size` must parse as u64. A malformed row aborts with InvalidRequest
///   (matching upstream `csv.Error -> ValidationError`).
pub fn parse_pulp_manifest(body: &str) -> Result<Vec<PulpManifestEntry>, ArtifactsError> {
    let mut out = Vec::new();
    for (idx, raw) in body.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.splitn(3, ',').collect();
        if cols.len() != 3 {
            return Err(ArtifactsError::InvalidRequest(format!(
                "PULP_MANIFEST line {}: expected 3 columns, got {}",
                idx + 1,
                cols.len()
            )));
        }
        let size: u64 = cols[2].parse().map_err(|_| {
            ArtifactsError::InvalidRequest(format!(
                "PULP_MANIFEST line {}: size '{}' is not a u64",
                idx + 1,
                cols[2]
            ))
        })?;
        if cols[0].is_empty() || cols[1].is_empty() {
            return Err(ArtifactsError::InvalidRequest(format!(
                "PULP_MANIFEST line {}: empty path or digest",
                idx + 1
            )));
        }
        out.push(PulpManifestEntry {
            relative_path: cols[0].to_string(),
            sha256: cols[1].to_string(),
            size,
        });
    }
    Ok(out)
}

/// Render PULP_MANIFEST CSV body from entries (round-trips parse_pulp_manifest).
pub fn render_pulp_manifest(entries: &[PulpManifestEntry]) -> String {
    let mut s = String::with_capacity(entries.len() * 96);
    for e in entries {
        s.push_str(&e.relative_path);
        s.push(',');
        s.push_str(&e.sha256);
        s.push(',');
        s.push_str(&e.size.to_string());
        s.push('\n');
    }
    s
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
        let entries: Vec<PulpManifestEntry> = units
            .iter()
            .map(|u| PulpManifestEntry {
                relative_path: u.relative_path.clone().unwrap_or_default(),
                sha256: u.sha256.clone().unwrap_or_default(),
                size: u.size.unwrap_or(0),
            })
            .collect();
        serde_json::json!({
            "PULP_MANIFEST": render_pulp_manifest(&entries),
            "count": entries.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file_content() {
        let plugin = FilePlugin;
        let unit = plugin.parse_content(b"hello world", "docs/readme.txt").unwrap();
        assert_eq!(unit.plugin_type, PluginType::File);
        assert_eq!(unit.relative_path.as_deref(), Some("docs/readme.txt"));
        assert!(unit.sha256.is_some());
        assert_eq!(unit.size, Some(11));
    }

    #[test]
    fn generate_file_manifest_csv() {
        let plugin = FilePlugin;
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/file/file/x/", 1);
        let units = vec![
            plugin.parse_content(b"a", "a.txt").unwrap(),
            plugin.parse_content(b"bb", "b.txt").unwrap(),
        ];
        let meta = plugin.generate_metadata(&ver, &units);
        let csv = meta["PULP_MANIFEST"].as_str().unwrap();
        assert_eq!(csv.lines().count(), 2);
        assert_eq!(meta["count"], 2);
    }
}

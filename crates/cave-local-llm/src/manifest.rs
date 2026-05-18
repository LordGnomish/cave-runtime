// SPDX-License-Identifier: AGPL-3.0-or-later
//! Parity manifest reader — wraps `cave_kernel::parity::ParityManifest` with filesystem I/O
//! and missing-function detection.

use cave_kernel::parity::ParityManifest;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest not found at {path}: {source}")]
    NotFound { path: PathBuf, source: std::io::Error },
    #[error("manifest parse error in {path}: {source}")]
    Parse { path: PathBuf, source: toml::de::Error },
}

pub type ManifestResult<T> = Result<T, ManifestError>;

/// Reads and parses `parity.manifest.toml` located at
/// `<workspace_root>/crates/<crate_name>/parity.manifest.toml`.
///
/// Returns the parsed manifest together with the crate root path so callers
/// can resolve relative source file paths.
pub fn read_crate_manifest(
    crate_name: &str,
    workspace_root: &Path,
) -> ManifestResult<(ParityManifest, PathBuf)> {
    let crate_root = workspace_root.join("crates").join(crate_name);
    let manifest_path = crate_root.join("parity.manifest.toml");

    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|source| ManifestError::NotFound { path: manifest_path.clone(), source })?;

    let manifest = toml::from_str::<ParityManifest>(&content)
        .map_err(|source| ManifestError::Parse { path: manifest_path.clone(), source })?;

    Ok((manifest, crate_root))
}

/// Reads and parses a `parity.manifest.toml` from an explicit file path.
pub fn parse_manifest_file(path: &Path) -> ManifestResult<ParityManifest> {
    let content = std::fs::read_to_string(path)
        .map_err(|source| ManifestError::NotFound { path: path.to_path_buf(), source })?;

    toml::from_str::<ParityManifest>(&content)
        .map_err(|source| ManifestError::Parse { path: path.to_path_buf(), source })
}

/// A `[[functions]]` entry whose `local_name` is absent from the declared source file.
#[derive(Debug, Clone)]
pub struct MissingFunction {
    pub upstream_name: String,
    pub local_name: String,
    /// Relative file path as declared in the manifest (e.g. `"src/routes.rs"`).
    pub file: String,
}

/// Returns every `[[functions]]` entry whose `fn <local_name>` signature cannot be found
/// in the corresponding source file under `crate_root`.
///
/// A file that cannot be read is treated as though the function is absent.
pub fn find_missing_functions(manifest: &ParityManifest, crate_root: &Path) -> Vec<MissingFunction> {
    manifest
        .functions
        .iter()
        .filter_map(|fm| {
            let abs_path = crate_root.join(&fm.file);
            let src = std::fs::read_to_string(&abs_path).unwrap_or_default();
            let needle = format!("fn {}", fm.local_name);
            if src.contains(&needle) {
                None
            } else {
                Some(MissingFunction {
                    upstream_name: fm.upstream_name.clone(),
                    local_name: fm.local_name.clone(),
                    file: fm.file.clone(),
                })
            }
        })
        .collect()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn sample_toml() -> &'static str {
        r#"
[upstream]
org     = "trufflesecurity"
repo    = "trufflehog"
version = "v3.63.0"

[module]
name        = "cave-secrets-test"
description = "unit test fixture"
source_root = "src"

[[functions]]
upstream_name = "ScanFile"
local_name    = "scan_file"
file          = "src/lib.rs"
"#
    }

    #[test]
    fn test_parse_manifest_file_round_trips() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(sample_toml().as_bytes()).unwrap();
        let manifest = parse_manifest_file(tmp.path()).unwrap();
        assert_eq!(manifest.primary_upstream().unwrap().org, "trufflesecurity");
        assert_eq!(manifest.module.name, "cave-secrets-test");
        assert_eq!(manifest.functions.len(), 1);
        assert_eq!(manifest.functions[0].local_name, "scan_file");
    }

    #[test]
    fn test_parse_manifest_file_missing_path_errors() {
        let result = parse_manifest_file(Path::new("/nonexistent/parity.manifest.toml"));
        assert!(matches!(result, Err(ManifestError::NotFound { .. })));
    }

    #[test]
    fn test_find_missing_functions_detects_present_and_absent() {
        let mut manifest_file = NamedTempFile::new().unwrap();
        manifest_file.write_all(sample_toml().as_bytes()).unwrap();
        let manifest = parse_manifest_file(manifest_file.path()).unwrap();

        // Write a source file that does NOT contain `fn scan_file`
        let tmp_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp_dir.path().join("src")).unwrap();
        std::fs::write(tmp_dir.path().join("src/lib.rs"), "// empty").unwrap();

        let missing = find_missing_functions(&manifest, tmp_dir.path());
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].local_name, "scan_file");

        // Now add the function
        std::fs::write(tmp_dir.path().join("src/lib.rs"), "pub fn scan_file() {}").unwrap();
        let missing = find_missing_functions(&manifest, tmp_dir.path());
        assert!(missing.is_empty());
    }
}

//! Integration tests for the manifest reader using real fixture files.

use cave_local_llm::manifest::{find_missing_functions, parse_manifest_file};
use cave_kernel::parity::ParityManifest;
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn load_test_manifest() -> ParityManifest {
    let path = fixture_dir().join("test_manifest.toml");
    parse_manifest_file(&path)
        .unwrap_or_else(|e| panic!("failed to parse test_manifest.toml: {e}"))
}

#[test]
fn test_manifest_upstream_fields() {
    let m = load_test_manifest();
    assert_eq!(m.upstream.org, "trufflesecurity");
    assert_eq!(m.upstream.repo, "trufflehog");
    assert_eq!(m.upstream.version, "v3.63.0");
}

#[test]
fn test_manifest_module_fields() {
    let m = load_test_manifest();
    assert_eq!(m.module.name, "cave-local-llm-test");
    assert!(m.module.description.is_some());
}

#[test]
fn test_manifest_has_two_function_mappings() {
    let m = load_test_manifest();
    assert_eq!(m.functions.len(), 2);

    let names: Vec<&str> = m.functions.iter().map(|f| f.local_name.as_str()).collect();
    assert!(names.contains(&"existing_fn"), "must include existing_fn");
    assert!(names.contains(&"missing_fn"), "must include missing_fn");
}

#[test]
fn test_manifest_has_no_surface_or_file_mappings() {
    let m = load_test_manifest();
    assert!(m.files.is_empty(), "fixture has no [[files]] mappings");
    assert!(m.surfaces.is_empty(), "fixture has no [[surfaces]] mappings");
    assert!(m.tests.is_empty(), "fixture has no [[tests]] mappings");
}

#[test]
fn test_find_missing_detects_exactly_one_absent_function() {
    let manifest = load_test_manifest();
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let missing = find_missing_functions(&manifest, crate_root);

    assert_eq!(missing.len(), 1, "only missing_fn should be absent; got: {missing:?}");
    assert_eq!(missing[0].local_name, "missing_fn");
    assert_eq!(missing[0].upstream_name, "MissingFunction");
}

#[test]
fn test_find_missing_returns_empty_when_all_present() {
    // Build a manifest that only references existing_fn (which is present)
    let toml_str = r#"
[upstream]
org     = "trufflesecurity"
repo    = "trufflehog"
version = "v3.63.0"

[module]
name = "t"

[[functions]]
upstream_name = "ExistingFunction"
local_name    = "existing_fn"
file          = "fixtures/sample_source.rs"
"#;
    let manifest: ParityManifest = toml::from_str(toml_str).unwrap();
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let missing = find_missing_functions(&manifest, crate_root);
    assert!(missing.is_empty(), "existing_fn is present — should not be missing");
}

#[test]
fn test_find_missing_treats_nonexistent_file_as_absent() {
    let toml_str = r#"
[upstream]
org     = "x"
repo    = "y"
version = "v1"

[module]
name = "t"

[[functions]]
upstream_name = "Foo"
local_name    = "foo"
file          = "src/nonexistent_file_xyz.rs"
"#;
    let manifest: ParityManifest = toml::from_str(toml_str).unwrap();
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let missing = find_missing_functions(&manifest, crate_root);
    assert_eq!(missing.len(), 1, "nonexistent file must count as missing");
    assert_eq!(missing[0].local_name, "foo");
}

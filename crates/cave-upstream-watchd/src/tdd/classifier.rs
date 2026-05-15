// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! File-path classifier: is this a test file, an impl file, or neither?
//!
//! The classifier is pure (path → kind) so it can be unit-tested without
//! touching git. It is intentionally conservative: a file that *contains*
//! tests inside an impl module (`#[cfg(test)] mod tests { ... }` inside
//! `src/foo.rs`) is classified as `ImplWithEmbeddedTests` and treated as a
//! mixed signal — `test_first` does not credit such files because the test
//! could not have landed in a separate commit from the impl.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// `tests/foo.rs`, `crates/X/tests/...`, `tests_*.rs`, `*_test.rs`.
    /// Pure test artifact — adding one without a corresponding src change
    /// is a credible "red" signal.
    Test,

    /// `src/foo.rs`, `src/bin/x.rs`, `build.rs` — implementation.
    /// A `#[cfg(test)] mod tests` block inside is recorded separately via
    /// `commit-time` content scanning, not here.
    Impl,

    /// Docs, manifests, config, CI, hooks, `Cargo.toml`, `parity.manifest`,
    /// READMEs. Neither test nor impl; ignored for TDD bookkeeping.
    NonCode,
}

/// Classify a single file path. Paths are normalised to forward slashes
/// first so the same rule applies on Windows-cloned repos.
pub fn classify_file<P: AsRef<Path>>(path: P) -> FileKind {
    let raw = path.as_ref().to_string_lossy().replace('\\', "/");

    // 1. Anything with `/tests/` in the path is a test file (cargo integration
    //    tests, examples-under-test, fixtures).
    if raw.contains("/tests/") || raw.starts_with("tests/") {
        return FileKind::Test;
    }

    // 2. File-name patterns: `_test.rs`, `_tests.rs`, `*_test.go` etc.
    if let Some(name) = Path::new(&raw).file_name().and_then(|n| n.to_str()) {
        if name.ends_with("_test.rs")
            || name.ends_with("_tests.rs")
            || name == "tests.rs"
            || name.ends_with("_test.go")
        {
            return FileKind::Test;
        }
    }

    // 3. Non-code: anything that isn't `.rs`, `.go`, `.ts`, `.tsx`, `.js`,
    //    `.py` is non-code. Cargo.toml, parity.manifest.toml, *.md, *.yml,
    //    *.yaml, *.json, *.sh, .gitignore, hook scripts all fall here.
    let is_code = raw.ends_with(".rs")
        || raw.ends_with(".go")
        || raw.ends_with(".ts")
        || raw.ends_with(".tsx")
        || raw.ends_with(".js")
        || raw.ends_with(".jsx")
        || raw.ends_with(".py");
    if !is_code {
        return FileKind::NonCode;
    }

    // 4. Otherwise it's implementation.
    FileKind::Impl
}

/// Derive a coarse "module" identifier from a file path. Used to group
/// commits — a test-only commit touching `crates/cave-foo/tests/bar.rs` is
/// considered to cover module `crates/cave-foo`. The grouping is
/// crate-level on purpose; a finer one (file-name stem) would too often
/// flag legitimate splits where a test for `foo` lives in `bar_test.rs`.
pub fn module_of<P: AsRef<Path>>(path: P) -> String {
    let raw = path.as_ref().to_string_lossy().replace('\\', "/");
    // crates/<name>/...
    if let Some(rest) = raw.strip_prefix("crates/") {
        if let Some(idx) = rest.find('/') {
            return format!("crates/{}", &rest[..idx]);
        }
        return format!("crates/{}", rest);
    }
    // scripts/, docs/, etc — bucket by top-level dir.
    if let Some(idx) = raw.find('/') {
        return raw[..idx].to_string();
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_dir_is_test() {
        assert_eq!(
            classify_file("crates/cave-foo/tests/integration.rs"),
            FileKind::Test
        );
        assert_eq!(classify_file("tests/smoke.rs"), FileKind::Test);
    }

    #[test]
    fn underscore_test_suffix_is_test() {
        assert_eq!(
            classify_file("crates/cave-foo/src/parser_test.rs"),
            FileKind::Test
        );
        assert_eq!(
            classify_file("crates/cave-foo/src/parser_tests.rs"),
            FileKind::Test
        );
        assert_eq!(classify_file("internal/tests.rs"), FileKind::Test);
    }

    #[test]
    fn go_test_suffix_is_test() {
        assert_eq!(classify_file("vendor/etcd/raft_test.go"), FileKind::Test);
    }

    #[test]
    fn src_rs_is_impl() {
        assert_eq!(
            classify_file("crates/cave-foo/src/lib.rs"),
            FileKind::Impl
        );
        assert_eq!(
            classify_file("crates/cave-foo/src/bin/main.rs"),
            FileKind::Impl
        );
    }

    #[test]
    fn cargo_toml_is_noncode() {
        assert_eq!(
            classify_file("crates/cave-foo/Cargo.toml"),
            FileKind::NonCode
        );
        assert_eq!(classify_file("Cargo.lock"), FileKind::NonCode);
    }

    #[test]
    fn manifest_and_docs_are_noncode() {
        assert_eq!(
            classify_file("crates/cave-foo/parity.manifest.toml"),
            FileKind::NonCode
        );
        assert_eq!(classify_file("README.md"), FileKind::NonCode);
        assert_eq!(classify_file("docs/adr/ADR-001.md"), FileKind::NonCode);
        assert_eq!(
            classify_file(".github/workflows/ci.yml"),
            FileKind::NonCode
        );
    }

    #[test]
    fn windows_backslash_paths_normalised() {
        assert_eq!(
            classify_file("crates\\cave-foo\\tests\\integration.rs"),
            FileKind::Test
        );
    }

    #[test]
    fn module_of_crate_path() {
        assert_eq!(
            module_of("crates/cave-foo/src/lib.rs"),
            "crates/cave-foo"
        );
        assert_eq!(
            module_of("crates/cave-foo/tests/bar.rs"),
            "crates/cave-foo"
        );
        assert_eq!(
            module_of("crates/cave-bar-baz/src/inner/m.rs"),
            "crates/cave-bar-baz"
        );
    }

    #[test]
    fn module_of_top_level() {
        assert_eq!(module_of("scripts/foo.sh"), "scripts");
        assert_eq!(module_of("docs/adr/X.md"), "docs");
        assert_eq!(module_of("README.md"), "README.md");
    }
}

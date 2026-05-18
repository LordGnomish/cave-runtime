// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests that drive `ShellGitInspector` against a real temp
//! git repo. They prove the gate works end-to-end with `git` CLI, not just
//! the mocked inspector.
//!
//! Each test seeds a fresh repo, makes a few commits matching a known TDD
//! shape, and asserts the gate output. The repo is dropped after the test.

use std::path::Path;
use std::process::Command;

use cave_upstream_watchd::tdd::{analyze_tdd_compliance, scan_stubs, ShellGitInspector};

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command");
    if !out.status.success() {
        panic!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn init_repo() -> tempfile::TempDir {
    let d = tempfile::TempDir::new().unwrap();
    git(d.path(), &["init", "-q", "-b", "main"]);
    git(d.path(), &["config", "user.email", "test@example.com"]);
    git(d.path(), &["config", "user.name", "Test"]);
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    // seed an initial commit on `main` so `main..branch` works.
    std::fs::write(d.path().join("README.md"), "init\n").unwrap();
    git(d.path(), &["add", "README.md"]);
    git(d.path(), &["commit", "-q", "-m", "init"]);
    d
}

fn commit_file(repo: &Path, path: &str, body: &str, subject: &str) {
    let full = repo.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&full, body).unwrap();
    git(repo, &["add", path]);
    git(repo, &["commit", "-q", "-m", subject]);
}

#[test]
fn real_git_pure_tdd_branch_passes() {
    let dir = init_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feat/foo"]);
    commit_file(
        dir.path(),
        "crates/cave-foo/tests/parser.rs",
        "#[test]\nfn t() { assert_eq!(2 + 2, 4); }\n",
        "test: parser red",
    );
    commit_file(
        dir.path(),
        "crates/cave-foo/src/parser.rs",
        "pub fn parse(s: &str) -> usize { s.len() }\n",
        "feat: parser green",
    );

    let inspector = ShellGitInspector::new(dir.path());
    let r = analyze_tdd_compliance(&inspector, "main", "feat/foo", true).unwrap();
    assert!(r.test_first, "violations: {:?}", r.details.violations);
    assert!(r.red_proof);
    assert!(r.green_proof);
    assert!(r.no_skip_attribute);
    assert!(r.is_pass());
    assert_eq!(r.details.commits.len(), 2);

    // stub scan finds zero stubs in the impl
    let stubs = scan_stubs(&inspector, "main", "feat/foo").unwrap();
    assert!(stubs.is_empty(), "{stubs:?}");
}

#[test]
fn real_git_mixed_commit_fails_test_first() {
    let dir = init_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feat/foo"]);

    // Single commit with both impl and test → Mixed
    std::fs::create_dir_all(dir.path().join("crates/cave-foo/src")).unwrap();
    std::fs::create_dir_all(dir.path().join("crates/cave-foo/tests")).unwrap();
    std::fs::write(
        dir.path().join("crates/cave-foo/src/parser.rs"),
        "pub fn parse() -> i32 { 1 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("crates/cave-foo/tests/parser.rs"),
        "#[test]\nfn t() {}\n",
    )
    .unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "feat+test together"]);

    let inspector = ShellGitInspector::new(dir.path());
    let r = analyze_tdd_compliance(&inspector, "main", "feat/foo", true).unwrap();
    assert!(!r.test_first);
    assert!(!r.red_proof); // no test-only commit
    assert!(!r.is_pass());
}

#[test]
fn real_git_stub_in_impl_is_detected() {
    let dir = init_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feat/foo"]);
    commit_file(
        dir.path(),
        "crates/cave-foo/tests/parser.rs",
        "#[test]\nfn t() {}\n",
        "test: red",
    );
    commit_file(
        dir.path(),
        "crates/cave-foo/src/parser.rs",
        "pub fn parse() -> i32 { todo!(\"later\") }\n",
        "feat: stubby",
    );

    let inspector = ShellGitInspector::new(dir.path());
    let stubs = scan_stubs(&inspector, "main", "feat/foo").unwrap();
    assert_eq!(stubs.len(), 1);
    assert!(stubs[0].path.ends_with("crates/cave-foo/src/parser.rs"));
}

#[test]
fn real_git_ignore_attribute_is_detected() {
    let dir = init_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feat/foo"]);
    commit_file(
        dir.path(),
        "crates/cave-foo/tests/parser.rs",
        "#[test]\n#[ignore = \"flaky\"]\nfn t() {}\n",
        "test: with ignore",
    );
    commit_file(
        dir.path(),
        "crates/cave-foo/src/parser.rs",
        "pub fn parse() -> i32 { 1 }\n",
        "feat: parser",
    );

    let inspector = ShellGitInspector::new(dir.path());
    let r = analyze_tdd_compliance(&inspector, "main", "feat/foo", true).unwrap();
    assert!(!r.no_skip_attribute);
    assert!(!r.is_pass());
}

#[test]
fn real_git_renamed_test_file_classified_correctly() {
    let dir = init_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feat/foo"]);
    commit_file(
        dir.path(),
        "crates/cave-foo/tests/old.rs",
        "#[test]\nfn t() {}\n",
        "test: initial",
    );
    // rename
    git(
        dir.path(),
        &[
            "mv",
            "crates/cave-foo/tests/old.rs",
            "crates/cave-foo/tests/new.rs",
        ],
    );
    git(dir.path(), &["commit", "-q", "-m", "test: rename"]);
    commit_file(
        dir.path(),
        "crates/cave-foo/src/parser.rs",
        "pub fn parse() -> i32 { 1 }\n",
        "feat: parser",
    );

    let inspector = ShellGitInspector::new(dir.path());
    let r = analyze_tdd_compliance(&inspector, "main", "feat/foo", true).unwrap();
    // Rename commit still counts as TestOnly since the renamed file is a test.
    assert!(r.test_first, "violations: {:?}", r.details.violations);
    assert!(r.is_pass());
}

#[test]
fn real_git_empty_branch_is_pass() {
    let dir = init_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feat/empty"]);
    // no commits beyond main
    let inspector = ShellGitInspector::new(dir.path());
    let r = analyze_tdd_compliance(&inspector, "main", "feat/empty", true).unwrap();
    assert!(r.is_pass());
    assert!(r.details.commits.is_empty());
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration: the auto-port dispatcher's `CharterV2Gate::verify_with_tdd`
//! folds the TDD analyzer's verdict into its existing `VerifyResult`.
//!
//! These tests use a real temp git repo + a real workspace with a
//! `parity.manifest.toml`, then drive the composite gate end-to-end.
//! `skip_cargo=true` keeps the test self-contained (no actual `cargo`
//! invocation against the fixture).

use std::fs;
use std::path::Path;
use std::process::Command;

use cave_upstream_watchd::auto_port_gate::{CharterBaseline, CharterV2Gate};
use cave_upstream_watchd::tdd::ShellGitInspector;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let d = tempfile::TempDir::new().unwrap();
    git(d.path(), &["init", "-q", "-b", "main"]);
    git(d.path(), &["config", "user.email", "t@t.t"]);
    git(d.path(), &["config", "user.name", "t"]);
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    // seed a parity manifest at the initial commit so the gate's
    // fill_ratio reader has something to read on `main`.
    let crate_dir = d.path().join("crates").join("cave-x");
    fs::create_dir_all(&crate_dir).unwrap();
    fs::write(
        crate_dir.join("parity.manifest.toml"),
        "[parity]\nfill_ratio = 0.7\nlast_audit = \"2026-05-13\"\n",
    )
    .unwrap();
    fs::write(d.path().join("README.md"), "init\n").unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-q", "-m", "init"]);
    d
}

fn bump_manifest(repo: &Path, ratio: f64) {
    fs::write(
        repo.join("crates")
            .join("cave-x")
            .join("parity.manifest.toml"),
        format!("[parity]\nfill_ratio = {ratio}\nlast_audit = \"2026-05-13\"\n"),
    )
    .unwrap();
}

#[tokio::test]
async fn verify_with_tdd_passes_when_branch_is_tdd_clean_and_ratio_rises() {
    let d = init_repo();
    git(d.path(), &["checkout", "-q", "-b", "feat/foo"]);

    // 1. test-only commit (red)
    fs::create_dir_all(d.path().join("crates/cave-x/tests")).unwrap();
    fs::write(
        d.path().join("crates/cave-x/tests/parser.rs"),
        "#[test]\nfn t() { assert!(true); }\n",
    )
    .unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-q", "-m", "test: red"]);

    // 2. impl-only commit (green) + bump manifest
    fs::create_dir_all(d.path().join("crates/cave-x/src")).unwrap();
    fs::write(
        d.path().join("crates/cave-x/src/parser.rs"),
        "pub fn parse() -> i32 { 1 }\n",
    )
    .unwrap();
    bump_manifest(d.path(), 0.8);
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-q", "-m", "feat: green"]);

    let gate = CharterV2Gate {
        workspace_root: d.path().to_path_buf(),
        cargo_path: "cargo".into(),
        skip_cargo: true,
    };
    let baseline = CharterBaseline {
        crate_name: "cave-x".into(),
        commit_sha_before: "0".repeat(40),
        fill_ratio_before: 0.7,
        workspace_stub_count_before: 0,
    };
    let inspector = ShellGitInspector::new(d.path());

    let r = gate
        .verify_with_tdd(&baseline, "sha-after", &inspector, "main", "feat/foo")
        .await
        .unwrap();

    let tdd = r.tdd_compliance.as_ref().expect("tdd_compliance populated");
    assert!(tdd.test_first, "violations: {:?}", tdd.details.violations);
    assert!(tdd.red_proof);
    assert!(tdd.green_proof);
    assert!(tdd.no_skip_attribute);
    assert!(tdd.is_pass());

    assert!(r.overall_pass, "notes: {:?}", r.notes);
    assert!(r.parity_ratio_delta > 0.0);
}

#[tokio::test]
async fn verify_with_tdd_fails_when_branch_is_mixed_commit() {
    let d = init_repo();
    git(d.path(), &["checkout", "-q", "-b", "feat/foo"]);

    // single Mixed commit — impl + tests together
    fs::create_dir_all(d.path().join("crates/cave-x/src")).unwrap();
    fs::create_dir_all(d.path().join("crates/cave-x/tests")).unwrap();
    fs::write(
        d.path().join("crates/cave-x/src/parser.rs"),
        "pub fn p() -> i32 { 1 }\n",
    )
    .unwrap();
    fs::write(
        d.path().join("crates/cave-x/tests/parser.rs"),
        "#[test]\nfn t() {}\n",
    )
    .unwrap();
    bump_manifest(d.path(), 0.8);
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-q", "-m", "feat+test together"]);

    let gate = CharterV2Gate {
        workspace_root: d.path().to_path_buf(),
        cargo_path: "cargo".into(),
        skip_cargo: true,
    };
    let baseline = CharterBaseline {
        crate_name: "cave-x".into(),
        commit_sha_before: "0".repeat(40),
        fill_ratio_before: 0.7,
        workspace_stub_count_before: 0,
    };
    let inspector = ShellGitInspector::new(d.path());

    let r = gate
        .verify_with_tdd(&baseline, "sha", &inspector, "main", "feat/foo")
        .await
        .unwrap();

    let tdd = r.tdd_compliance.as_ref().unwrap();
    assert!(!tdd.is_pass());
    // The TDD failure must NOT be papered over by the otherwise-clean
    // ratio / stub gates — the composite gate must reject this branch.
    assert!(
        !r.overall_pass,
        "composite gate let a mixed-commit branch through: notes={:?}",
        r.notes
    );
    assert!(
        r.notes.iter().any(|n| n.contains("TDD-strict")),
        "TDD failure note missing: {:?}",
        r.notes
    );
}

#[tokio::test]
async fn verify_with_tdd_fails_when_ignore_attr_present() {
    let d = init_repo();
    git(d.path(), &["checkout", "-q", "-b", "feat/foo"]);

    fs::create_dir_all(d.path().join("crates/cave-x/tests")).unwrap();
    fs::write(
        d.path().join("crates/cave-x/tests/parser.rs"),
        "#[test]\n#[ignore = \"flaky\"]\nfn t() {}\n",
    )
    .unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-q", "-m", "test: with ignore"]);

    fs::create_dir_all(d.path().join("crates/cave-x/src")).unwrap();
    fs::write(
        d.path().join("crates/cave-x/src/parser.rs"),
        "pub fn p() -> i32 { 1 }\n",
    )
    .unwrap();
    bump_manifest(d.path(), 0.8);
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-q", "-m", "feat: parser"]);

    let gate = CharterV2Gate {
        workspace_root: d.path().to_path_buf(),
        cargo_path: "cargo".into(),
        skip_cargo: true,
    };
    let baseline = CharterBaseline {
        crate_name: "cave-x".into(),
        commit_sha_before: "0".repeat(40),
        fill_ratio_before: 0.7,
        workspace_stub_count_before: 0,
    };
    let inspector = ShellGitInspector::new(d.path());

    let r = gate
        .verify_with_tdd(&baseline, "sha", &inspector, "main", "feat/foo")
        .await
        .unwrap();

    let tdd = r.tdd_compliance.as_ref().unwrap();
    assert!(!tdd.no_skip_attribute);
    assert!(!r.overall_pass);
}

#[tokio::test]
async fn legacy_verify_path_leaves_tdd_compliance_none() {
    // Backwards-compat: the original `verify()` (without TDD) still
    // returns `tdd_compliance: None` so existing callers see no
    // semantic change.
    let d = init_repo();
    bump_manifest(d.path(), 0.8); // ensure ratio rose

    let gate = CharterV2Gate {
        workspace_root: d.path().to_path_buf(),
        cargo_path: "cargo".into(),
        skip_cargo: true,
    };
    let baseline = CharterBaseline {
        crate_name: "cave-x".into(),
        commit_sha_before: "0".repeat(40),
        fill_ratio_before: 0.7,
        workspace_stub_count_before: 0,
    };

    use cave_upstream_watchd::auto_port_gate::CharterGate;
    let r = gate.verify(&baseline, "sha").await.unwrap();
    assert!(r.tdd_compliance.is_none());
    assert!(r.overall_pass, "notes: {:?}", r.notes);
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Walks a branch's commit history and computes [`TddCompliance`].
//!
//! The four signals:
//!
//! | signal             | how it is derived                                              |
//! |--------------------|---------------------------------------------------------------|
//! | `test_first`       | every `ImplOnly` or `Mixed` commit must have a prior          |
//! |                    | `TestOnly` commit on the same module bucket                   |
//! | `red_proof`        | branch contains ≥1 `TestOnly` commit (heuristic stand-in for  |
//! |                    | "test was red before impl"; runtime verification optional)    |
//! | `green_proof`      | supplied externally — typically the result of `cargo test`    |
//! | `no_skip_attribute`| no `#[ignore]` attribute in any changed test file at branch   |
//! |                    | tip                                                           |
//!
//! ## Why a heuristic for `red_proof`?
//!
//! True red verification would mean checking out each test-only commit and
//! running `cargo test --no-run` on it to confirm it fails to compile (the
//! test references an impl symbol not yet defined). That is expensive
//! (~minutes per commit) and brittle (build deps may not resolve cleanly
//! on every historical commit). The heuristic — "a test-only commit was
//! made before impl" — is the observable trace of a red→green cycle the
//! author *actually performed*. A team that squashes red→green pairs into
//! one commit forfeits the heuristic; that is a Charter §1 choice they
//! make. The CLI exposes `--verify-red-runtime` for the heavyweight check.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::tdd::classifier::{FileKind, classify_file, module_of};
use crate::tdd::git_inspector::GitInspector;
use crate::tdd::{ClassifiedCommit, CommitKind, TddCompliance, TddDetails, TddError, TddFinding};

/// Stateless façade. Convenience wrapper around [`analyze_tdd_compliance`].
pub struct TddAnalyzer<'a, I: GitInspector + ?Sized> {
    inspector: &'a I,
}

impl<'a, I: GitInspector + ?Sized> TddAnalyzer<'a, I> {
    pub fn new(inspector: &'a I) -> Self {
        Self { inspector }
    }

    /// Run the TDD analysis end-to-end.
    ///
    /// `green_proof` is supplied by the caller because verifying it requires
    /// invoking the test runner, which sits outside this crate's scope.
    pub fn analyze(
        &self,
        base: &str,
        branch: &str,
        green_proof: bool,
    ) -> Result<TddCompliance, TddError> {
        analyze_tdd_compliance(self.inspector, base, branch, green_proof)
    }
}

pub fn analyze_tdd_compliance<I: GitInspector + ?Sized>(
    inspector: &I,
    base: &str,
    branch: &str,
    green_proof: bool,
) -> Result<TddCompliance, TddError> {
    let raw_commits = inspector
        .commits_between(base, branch)
        .map_err(TddError::Git)?;

    // 1. classify every commit
    let classified: Vec<ClassifiedCommit> =
        raw_commits.iter().map(|c| classify_commit(c)).collect();

    // 2. test_first — module-by-module ordering
    let mut violations = Vec::new();
    let mut covered_modules: HashSet<String> = HashSet::new();
    let mut impl_count = 0u32;
    let mut test_only_count = 0u32;
    let mut mixed_count = 0u32;

    for c in &classified {
        match c.kind {
            CommitKind::TestOnly => {
                test_only_count += 1;
                for m in &c.touched_modules {
                    covered_modules.insert(m.clone());
                }
            }
            CommitKind::ImplOnly => {
                impl_count += 1;
                for m in &c.touched_modules {
                    if !covered_modules.contains(m) {
                        violations.push(TddFinding::ImplWithoutPriorTest {
                            impl_sha: c.sha.clone(),
                            module: m.clone(),
                        });
                    }
                }
            }
            CommitKind::Mixed => {
                mixed_count += 1;
                impl_count += 1;
                // Mixed commits "self-cover": the test arrived with the
                // impl, not before it. Record as a finding (not a hard
                // failure on its own) but do not retroactively count this
                // module as test-first–covered.
                violations.push(TddFinding::MixedCommit {
                    sha: c.sha.clone(),
                    modules: c.touched_modules.clone(),
                });
            }
            CommitKind::NonCode => {}
        }
    }

    // `test_first` passes if: at least one impl commit landed AND every
    // ImplOnly commit found prior test coverage AND there were no Mixed
    // commits. If no impl commits exist at all (e.g. branch only changes
    // docs), test_first is trivially true.
    let test_first = if impl_count == 0 {
        true
    } else {
        let impl_without_prior = violations
            .iter()
            .any(|v| matches!(v, TddFinding::ImplWithoutPriorTest { .. }));
        !impl_without_prior && mixed_count == 0
    };

    // 3. red_proof — at least one test-only commit observed (and any impl
    //    work exists). If branch is doc-only, red is trivially true.
    let red_proof = if impl_count == 0 {
        true
    } else {
        test_only_count >= 1
    };

    // 4. no_skip_attribute — scan every test file currently in branch
    let ignore_findings = scan_for_ignore_attr(inspector, branch, &raw_commits)?;
    let no_skip_attribute = ignore_findings.is_empty();
    for f in ignore_findings {
        violations.push(f);
    }

    Ok(TddCompliance {
        test_first,
        red_proof,
        green_proof,
        no_skip_attribute,
        details: TddDetails {
            commits: classified,
            violations,
        },
    })
}

fn classify_commit(c: &crate::tdd::git_inspector::CommitInfo) -> ClassifiedCommit {
    let mut has_test = false;
    let mut has_impl = false;
    let mut has_noncode = false;
    let mut modules: Vec<String> = Vec::new();

    for f in &c.files {
        match classify_file(&f.path) {
            FileKind::Test => {
                has_test = true;
                let m = module_of(&f.path);
                if !modules.contains(&m) {
                    modules.push(m);
                }
            }
            FileKind::Impl => {
                has_impl = true;
                let m = module_of(&f.path);
                if !modules.contains(&m) {
                    modules.push(m);
                }
            }
            FileKind::NonCode => {
                has_noncode = true;
            }
        }
    }

    let kind = match (has_test, has_impl) {
        (true, false) => CommitKind::TestOnly,
        (false, true) => CommitKind::ImplOnly,
        (true, true) => CommitKind::Mixed,
        (false, false) => {
            if has_noncode {
                CommitKind::NonCode
            } else {
                CommitKind::NonCode
            }
        }
    };

    ClassifiedCommit {
        sha: c.sha.clone(),
        subject: c.subject.clone(),
        kind,
        touched_modules: modules,
    }
}

fn scan_for_ignore_attr<I: GitInspector + ?Sized>(
    inspector: &I,
    branch: &str,
    commits: &[crate::tdd::git_inspector::CommitInfo],
) -> Result<Vec<TddFinding>, TddError> {
    // Collect unique test-file paths touched by any commit on the branch,
    // then read each at branch tip and grep for `#[ignore]`.
    let mut test_paths: HashMap<String, std::path::PathBuf> = HashMap::new();
    for c in commits {
        for f in &c.files {
            if classify_file(&f.path) == FileKind::Test {
                let key = f.path.to_string_lossy().to_string();
                test_paths.insert(key, f.path.clone());
            }
        }
    }

    let mut findings = Vec::new();
    for path in test_paths.values() {
        let body = match inspector
            .read_at_commit(branch, path)
            .map_err(TddError::Git)?
        {
            Some(b) => b,
            None => continue,
        };
        findings.extend(scan_ignore_in_body(path, &body));
    }
    Ok(findings)
}

/// Public so the pre-commit hook can reuse it.
pub fn scan_ignore_in_body(path: &Path, body: &str) -> Vec<TddFinding> {
    let mut out = Vec::new();
    for (idx, line) in body.lines().enumerate() {
        let stripped = line.trim_start();
        if stripped.starts_with("//") {
            continue;
        }
        // Match `#[ignore]` or `#[ignore = "..."]` or `#[ignore(...)]`.
        // Use a hand-rolled check rather than regex to keep this lean.
        if (stripped.starts_with("#[ignore]")
            || stripped.starts_with("#[ignore ")
            || stripped.starts_with("#[ignore="))
            || (stripped.starts_with("#[ignore(") && stripped.contains(')'))
        {
            out.push(TddFinding::IgnoreAttribute {
                path: path.to_path_buf(),
                line: idx + 1,
                snippet: line.trim().to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tdd::git_inspector::{CommitInfo, FileChange, FileChangeKind, GitError};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// In-memory mock inspector — supplies commits and per-path content.
    struct MockInspector {
        commits: Vec<CommitInfo>,
        /// keyed by "sha::path"
        content: HashMap<String, String>,
        /// branch-tip content lookups
        tip_content: HashMap<String, String>,
        log: Mutex<Vec<(String, String)>>,
    }

    impl MockInspector {
        fn new() -> Self {
            Self {
                commits: Vec::new(),
                content: HashMap::new(),
                tip_content: HashMap::new(),
                log: Mutex::new(Vec::new()),
            }
        }

        fn add_commit(&mut self, sha: &str, subject: &str, files: Vec<(&str, FileChangeKind)>) {
            self.commits.push(CommitInfo {
                sha: sha.to_string(),
                subject: subject.to_string(),
                files: files
                    .into_iter()
                    .map(|(p, k)| FileChange {
                        path: PathBuf::from(p),
                        kind: k,
                    })
                    .collect(),
            });
        }

        fn set_tip(&mut self, branch: &str, path: &str, body: &str) {
            self.tip_content
                .insert(format!("{}::{}", branch, path), body.to_string());
        }
    }

    impl GitInspector for MockInspector {
        fn commits_between(&self, base: &str, branch: &str) -> Result<Vec<CommitInfo>, GitError> {
            self.log
                .lock()
                .unwrap()
                .push((base.to_string(), branch.to_string()));
            Ok(self.commits.clone())
        }

        fn read_at_commit(&self, sha: &str, path: &Path) -> Result<Option<String>, GitError> {
            let k = format!("{}::{}", sha, path.display());
            if let Some(c) = self.tip_content.get(&k) {
                return Ok(Some(c.clone()));
            }
            Ok(self.content.get(&k).cloned())
        }
    }

    #[test]
    fn pure_tdd_branch_passes() {
        // 1. test-only commit, 2. impl commit — classic red→green.
        let mut m = MockInspector::new();
        m.add_commit(
            "AAA",
            "test: add foo parser tests",
            vec![("crates/cave-foo/tests/parser.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "BBB",
            "feat: foo parser",
            vec![("crates/cave-foo/src/parser.rs", FileChangeKind::Added)],
        );
        m.set_tip("branch", "crates/cave-foo/tests/parser.rs", "fn t() {}\n");

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(r.test_first, "details: {:?}", r.details.violations);
        assert!(r.red_proof);
        assert!(r.green_proof);
        assert!(r.no_skip_attribute);
        assert!(r.is_pass());
    }

    #[test]
    fn mixed_commit_fails_test_first() {
        // Impl + test in same commit — Charter §1 violation.
        let mut m = MockInspector::new();
        m.add_commit(
            "AAA",
            "feat+test: foo parser",
            vec![
                ("crates/cave-foo/src/parser.rs", FileChangeKind::Added),
                ("crates/cave-foo/tests/parser.rs", FileChangeKind::Added),
            ],
        );
        m.set_tip("branch", "crates/cave-foo/tests/parser.rs", "fn t() {}\n");

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(!r.test_first);
        // red_proof also fails — there is no test-only commit.
        assert!(!r.red_proof);
        assert!(r.green_proof);
        assert!(!r.is_pass());
        let has_mixed = r
            .details
            .violations
            .iter()
            .any(|v| matches!(v, TddFinding::MixedCommit { .. }));
        assert!(has_mixed);
    }

    #[test]
    fn impl_first_then_test_fails_test_first() {
        let mut m = MockInspector::new();
        m.add_commit(
            "AAA",
            "feat: foo parser",
            vec![("crates/cave-foo/src/parser.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "BBB",
            "test: add tests",
            vec![("crates/cave-foo/tests/parser.rs", FileChangeKind::Added)],
        );
        m.set_tip("branch", "crates/cave-foo/tests/parser.rs", "fn t() {}\n");

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(!r.test_first);
        // Test-only commit *did* land, but it landed after impl. red_proof
        // still triggers on count alone — that is the heuristic's known
        // weakness; test_first is the stricter signal.
        assert!(r.red_proof);
        assert!(!r.is_pass());
    }

    #[test]
    fn doc_only_branch_is_trivially_compliant() {
        let mut m = MockInspector::new();
        m.add_commit(
            "AAA",
            "docs: update README",
            vec![("README.md", FileChangeKind::Modified)],
        );

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(r.test_first);
        assert!(r.red_proof);
        assert!(r.no_skip_attribute);
        assert!(r.is_pass());
    }

    #[test]
    fn green_proof_passthrough() {
        let mut m = MockInspector::new();
        m.add_commit(
            "AAA",
            "test: add tests",
            vec![("crates/cave-foo/tests/parser.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "BBB",
            "feat: parser",
            vec![("crates/cave-foo/src/parser.rs", FileChangeKind::Added)],
        );
        m.set_tip("branch", "crates/cave-foo/tests/parser.rs", "fn t() {}\n");

        let r = analyze_tdd_compliance(&m, "main", "branch", false).unwrap();
        assert!(r.test_first);
        assert!(r.red_proof);
        assert!(!r.green_proof);
        assert!(!r.is_pass()); // green_proof false fails composite
    }

    #[test]
    fn ignore_attribute_caught() {
        let mut m = MockInspector::new();
        m.add_commit(
            "AAA",
            "test: tests",
            vec![("crates/cave-foo/tests/parser.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "BBB",
            "feat: parser",
            vec![("crates/cave-foo/src/parser.rs", FileChangeKind::Added)],
        );
        m.set_tip(
            "branch",
            "crates/cave-foo/tests/parser.rs",
            "#[test]\n#[ignore]\nfn t() {}\n",
        );

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(!r.no_skip_attribute);
        let found = r
            .details
            .violations
            .iter()
            .any(|v| matches!(v, TddFinding::IgnoreAttribute { line, .. } if *line == 2));
        assert!(found, "ignore not surfaced: {:?}", r.details.violations);
    }

    #[test]
    fn ignore_with_reason_caught() {
        let mut m = MockInspector::new();
        m.add_commit(
            "AAA",
            "test: tests",
            vec![("crates/cave-foo/tests/parser.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "BBB",
            "feat: parser",
            vec![("crates/cave-foo/src/parser.rs", FileChangeKind::Added)],
        );
        m.set_tip(
            "branch",
            "crates/cave-foo/tests/parser.rs",
            "#[test]\n#[ignore = \"flaky on CI\"]\nfn t() {}\n",
        );

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(!r.no_skip_attribute);
    }

    #[test]
    fn ignore_in_comment_not_flagged() {
        // `// #[ignore]` should not trigger.
        let body = "fn t() {} // we used to write #[ignore] here\n";
        let v = scan_ignore_in_body(Path::new("x.rs"), body);
        assert!(v.is_empty());
    }

    #[test]
    fn ignore_with_args_caught() {
        let body = "#[ignore(reason = \"slow\")]\nfn t() {}\n";
        let v = scan_ignore_in_body(Path::new("x.rs"), body);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn two_modules_test_first_covered_independently() {
        let mut m = MockInspector::new();
        m.add_commit(
            "A",
            "test: foo",
            vec![("crates/cave-foo/tests/x.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "B",
            "feat: foo",
            vec![("crates/cave-foo/src/x.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "C",
            "test: bar",
            vec![("crates/cave-bar/tests/y.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "D",
            "feat: bar",
            vec![("crates/cave-bar/src/y.rs", FileChangeKind::Added)],
        );
        m.set_tip("branch", "crates/cave-foo/tests/x.rs", "fn t() {}\n");
        m.set_tip("branch", "crates/cave-bar/tests/y.rs", "fn t() {}\n");

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(r.test_first);
        assert!(r.is_pass());
    }

    #[test]
    fn impl_for_module_without_prior_test_caught() {
        // Test exists for module A, but impl lands for module B without
        // its own test-only commit.
        let mut m = MockInspector::new();
        m.add_commit(
            "A",
            "test: foo",
            vec![("crates/cave-foo/tests/x.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "B",
            "feat: foo",
            vec![("crates/cave-foo/src/x.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "C",
            "feat: bar",
            vec![("crates/cave-bar/src/y.rs", FileChangeKind::Added)],
        );
        m.set_tip("branch", "crates/cave-foo/tests/x.rs", "fn t() {}\n");

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(!r.test_first);
        let bar_violation = r.details.violations.iter().any(|v| {
            matches!(
                v,
                TddFinding::ImplWithoutPriorTest { module, .. } if module == "crates/cave-bar"
            )
        });
        assert!(bar_violation);
    }

    #[test]
    fn noncode_commits_dont_break_test_first() {
        let mut m = MockInspector::new();
        m.add_commit("A", "docs", vec![("README.md", FileChangeKind::Modified)]);
        m.add_commit(
            "B",
            "test: foo",
            vec![("crates/cave-foo/tests/x.rs", FileChangeKind::Added)],
        );
        m.add_commit(
            "C",
            "manifest",
            vec![("Cargo.toml", FileChangeKind::Modified)],
        );
        m.add_commit(
            "D",
            "feat: foo",
            vec![("crates/cave-foo/src/x.rs", FileChangeKind::Added)],
        );
        m.set_tip("branch", "crates/cave-foo/tests/x.rs", "fn t() {}\n");

        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(r.test_first);
        assert!(r.is_pass());
    }

    #[test]
    fn empty_branch_passes_trivially() {
        let m = MockInspector::new();
        let r = analyze_tdd_compliance(&m, "main", "branch", true).unwrap();
        assert!(r.is_pass());
    }
}

//! TDD-strict mode analyzer — the Charter Golden Rule §1
//! ("line-by-line TDD upstream parity") turned into a verifiable signal.
//!
//! This module is the new core of Charter v2 TDD compliance. It walks a
//! branch's commit history and produces a [`TddCompliance`] verdict on four
//! axes:
//!
//! | signal             | how it is derived                                           |
//! |--------------------|-------------------------------------------------------------|
//! | `test_first`       | every impl-touching commit had a prior test-only commit on  |
//! |                    | the same module                                             |
//! | `red_proof`        | the branch contains ≥ 1 test-only commit                    |
//! | `green_proof`      | supplied externally (CI feeds `cargo test` result)          |
//! | `no_skip_attribute`| no `#[ignore]` attribute in any changed test file at tip    |
//!
//! The signal is consumed two ways:
//!
//!   1. [`crate::auto_port_gate::CharterV2Gate::verify`] embeds the result
//!      in its [`VerifyResult`](crate::auto_port_gate::VerifyResult) when a
//!      [`GitInspector`] + base ref is supplied, so the auto-port
//!      dispatcher refuses to merge non-TDD work.
//!
//!   2. The `cave-tdd-check` CLI runs the analyzer directly against a
//!      branch — used in CI on every PR and locally before push.
//!
//! ## Why not enforce red-runtime by default?
//!
//! True red verification would mean checking out each test-only commit and
//! running `cargo test --no-run` to confirm a compile error. That is
//! expensive (~minutes per commit) and brittle (build deps may not resolve
//! cleanly on every historical commit). The heuristic — "a test-only
//! commit landed before the impl" — is the observable trace of a red→green
//! cycle the author *actually performed*. A team that squashes red→green
//! pairs into one commit forfeits the heuristic; that is the Charter §1
//! discipline they trade off. A heavyweight runtime check is intentionally
//! out of scope for this layer.

pub mod classifier;
pub mod git_inspector;
pub mod stub_scan;
pub mod tdd_analyzer;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use classifier::{classify_file, FileKind};
pub use git_inspector::{
    CommitInfo, FileChange, FileChangeKind, GitError, GitInspector, ShellGitInspector,
};
pub use stub_scan::{scan_path, scan_stubs, StubFinding, StubKind};
pub use tdd_analyzer::{analyze_tdd_compliance, scan_ignore_in_body, TddAnalyzer};

/// TDD-strict mode verdict. All four booleans must be true for the gate to
/// pass; `details` records the supporting evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TddCompliance {
    pub test_first: bool,
    pub red_proof: bool,
    pub green_proof: bool,
    pub no_skip_attribute: bool,
    pub details: TddDetails,
}

impl TddCompliance {
    pub fn is_pass(&self) -> bool {
        self.test_first && self.red_proof && self.green_proof && self.no_skip_attribute
    }

    /// One-line summary suitable for CI logs.
    pub fn summary(&self) -> String {
        fn m(b: bool, label: &str) -> String {
            if b {
                format!("{label}=ok")
            } else {
                format!("{label}=FAIL")
            }
        }
        format!(
            "{} {} {} {}",
            m(self.test_first, "test_first"),
            m(self.red_proof, "red"),
            m(self.green_proof, "green"),
            m(self.no_skip_attribute, "no_ignore"),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TddDetails {
    pub commits: Vec<ClassifiedCommit>,
    pub violations: Vec<TddFinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifiedCommit {
    pub sha: String,
    pub subject: String,
    pub kind: CommitKind,
    pub touched_modules: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitKind {
    TestOnly,
    ImplOnly,
    Mixed,
    NonCode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TddFinding {
    ImplWithoutPriorTest {
        impl_sha: String,
        module: String,
    },
    IgnoreAttribute {
        path: PathBuf,
        line: usize,
        snippet: String,
    },
    MixedCommit {
        sha: String,
        modules: Vec<String>,
    },
}

/// Error type for the TDD analyzer.
#[derive(Debug, thiserror::Error)]
pub enum TddError {
    #[error("git error: {0}")]
    Git(#[from] GitError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

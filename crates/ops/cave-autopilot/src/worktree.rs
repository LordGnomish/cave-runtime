// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-task git worktree lifecycle.
//!
//! Each dispatched task runs in an isolated `git worktree` so concurrent jobs
//! never clobber each other and a failed attempt leaves the main checkout
//! untouched. The flow is:
//!
//! ```text
//!   git worktree add  →  (LLM writes code)  →  cargo build  →  cargo test
//!     PASS → git commit → git merge --no-ff into the daemon branch
//!     FAIL → iterate (escalation ladder) or, after exhaustion, abandon
//! ```
//!
//! **Push is never performed here** — merging stays local; shipping to a remote
//! is a human gate (Burak's explicit permission).
//!
//! Command *argument construction* and output *parsing* are pure functions; the
//! methods that actually spawn `git`/`cargo` are thin wrappers over them.

use crate::error::{AutopilotError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of running an external command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutcome {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// An isolated worktree bound to one task.
#[derive(Debug, Clone)]
pub struct WorktreeJob {
    pub repo_root: PathBuf,
    pub worktree_path: PathBuf,
    pub branch: String,
    pub task_id: String,
}

impl WorktreeJob {
    /// Derive the branch + worktree path for a task. Branch convention:
    /// `autopilot/<task_id>`; worktree under `worktree_root/<task_id>`.
    pub fn new(repo_root: &Path, worktree_root: &Path, task_id: &str) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            worktree_path: worktree_root.join(task_id),
            branch: format!("autopilot/{task_id}"),
            task_id: task_id.to_string(),
        }
    }

    // ---- pure argument builders (unit-tested) ----

    /// `git worktree add -b <branch> <path> <base>`.
    pub fn add_args(&self, base: &str) -> Vec<String> {
        vec![
            "worktree".into(),
            "add".into(),
            "-b".into(),
            self.branch.clone(),
            self.worktree_path.to_string_lossy().into_owned(),
            base.into(),
        ]
    }

    /// `git worktree remove --force <path>`.
    pub fn remove_args(&self) -> Vec<String> {
        vec![
            "worktree".into(),
            "remove".into(),
            "--force".into(),
            self.worktree_path.to_string_lossy().into_owned(),
        ]
    }

    /// `git merge --no-ff <branch> -m <msg>` — run from the daemon branch in the
    /// main checkout. Never followed by a push.
    pub fn merge_no_ff_args(&self, msg: &str) -> Vec<String> {
        vec![
            "merge".into(),
            "--no-ff".into(),
            self.branch.clone(),
            "-m".into(),
            msg.into(),
        ]
    }

    /// `cargo test -p <crate>` argument vector.
    pub fn cargo_test_args(crate_name: &str) -> Vec<String> {
        vec!["test".into(), "-p".into(), crate_name.into()]
    }

    /// `cargo build -p <crate>` argument vector.
    pub fn cargo_build_args(crate_name: &str) -> Vec<String> {
        vec!["build".into(), "-p".into(), crate_name.into()]
    }

    // ---- pure output parsers ----

    /// True iff cargo test output reports success and no failures. We require at
    /// least one `test result: ok.` line and zero `test result: FAILED` lines,
    /// so an all-skipped or non-test invocation is *not* treated as a pass.
    pub fn tests_passed(output: &str) -> bool {
        let has_ok = output.contains("test result: ok.");
        let has_fail = output.contains("test result: FAILED");
        has_ok && !has_fail
    }

    // ---- I/O methods ----

    fn run(dir: &Path, program: &str, args: &[String]) -> Result<CmdOutcome> {
        let out = Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|e| AutopilotError::Worktree(format!("spawn {program}: {e}")))?;
        Ok(CmdOutcome {
            success: out.status.success(),
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Create the worktree off `base` (e.g. `"HEAD"` or `"main"`).
    pub fn create(&self, base: &str) -> Result<CmdOutcome> {
        if let Some(parent) = self.worktree_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::run(&self.repo_root, "git", &self.add_args(base))
    }

    /// `cargo build` inside the worktree for one crate.
    pub fn build(&self, crate_name: &str) -> Result<CmdOutcome> {
        Self::run(&self.worktree_path, "cargo", &Self::cargo_build_args(crate_name))
    }

    /// `cargo test` inside the worktree for one crate.
    pub fn test(&self, crate_name: &str) -> Result<CmdOutcome> {
        Self::run(&self.worktree_path, "cargo", &Self::cargo_test_args(crate_name))
    }

    /// Stage everything and commit inside the worktree.
    pub fn commit_all(&self, message: &str) -> Result<CmdOutcome> {
        let add = Self::run(&self.worktree_path, "git", &["add".into(), "-A".into()])?;
        if !add.success {
            return Ok(add);
        }
        Self::run(
            &self.worktree_path,
            "git",
            &["commit".into(), "-m".into(), message.into()],
        )
    }

    /// Merge the task branch into the current branch of the main checkout,
    /// no-fast-forward, no push.
    pub fn merge_no_ff(&self, message: &str) -> Result<CmdOutcome> {
        Self::run(&self.repo_root, "git", &self.merge_no_ff_args(message))
    }

    /// Tear down the worktree (does not delete the branch).
    pub fn remove(&self) -> Result<CmdOutcome> {
        Self::run(&self.repo_root, "git", &self.remove_args())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> WorktreeJob {
        WorktreeJob::new(
            Path::new("/repo"),
            Path::new("/repo/.autopilot/worktrees"),
            "port-cave-etcd",
        )
    }

    #[test]
    fn branch_and_path_conventions() {
        let j = job();
        assert_eq!(j.branch, "autopilot/port-cave-etcd");
        assert_eq!(
            j.worktree_path,
            PathBuf::from("/repo/.autopilot/worktrees/port-cave-etcd")
        );
    }

    #[test]
    fn add_args_use_new_branch_and_base() {
        let a = job().add_args("HEAD");
        assert_eq!(a[0], "worktree");
        assert_eq!(a[1], "add");
        assert_eq!(a[2], "-b");
        assert_eq!(a[3], "autopilot/port-cave-etcd");
        assert_eq!(a.last().unwrap(), "HEAD");
    }

    #[test]
    fn merge_is_no_ff() {
        let a = job().merge_no_ff_args("merge: autopilot port-cave-etcd");
        assert_eq!(a[0], "merge");
        assert_eq!(a[1], "--no-ff");
        assert_eq!(a[2], "autopilot/port-cave-etcd");
    }

    #[test]
    fn cargo_arg_builders() {
        assert_eq!(
            WorktreeJob::cargo_test_args("cave-test"),
            vec!["test", "-p", "cave-test"]
        );
        assert_eq!(
            WorktreeJob::cargo_build_args("cave-test"),
            vec!["build", "-p", "cave-test"]
        );
    }

    #[test]
    fn tests_passed_requires_ok_and_no_failures() {
        assert!(WorktreeJob::tests_passed(
            "running 5 tests\ntest result: ok. 5 passed; 0 failed"
        ));
        assert!(!WorktreeJob::tests_passed(
            "test result: ok. 3 passed\ntest result: FAILED. 1 passed; 1 failed"
        ));
        // No tests at all is not a pass.
        assert!(!WorktreeJob::tests_passed("Compiling cave-test v0.1.0"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Git inspection trait and a `git` CLI–backed implementation.
//!
//! Trait is small enough to mock in unit tests; the real implementation
//! shells out to `git` (already a system dependency for the repo).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct CommitInfo {
    pub sha: String,
    pub subject: String,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Other,
}

impl FileChangeKind {
    fn from_status(s: &str) -> Self {
        match s.chars().next() {
            Some('A') => FileChangeKind::Added,
            Some('M') => FileChangeKind::Modified,
            Some('D') => FileChangeKind::Deleted,
            Some('R') => FileChangeKind::Renamed,
            _ => FileChangeKind::Other,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git command failed: {0}")]
    Command(String),

    #[error("git output parse error: {0}")]
    Parse(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Minimal git surface used by [`TddAnalyzer`]. Mockable for tests.
pub trait GitInspector: Send + Sync {
    /// Return commits reachable from `branch` but not from `base`,
    /// oldest-first. Empty vec if branch is at or behind base.
    fn commits_between(&self, base: &str, branch: &str) -> Result<Vec<CommitInfo>, GitError>;

    /// Read a file as it exists at `sha`. Returns Ok(None) if the path
    /// does not exist at that commit (e.g. it was added later).
    fn read_at_commit(&self, sha: &str, path: &Path) -> Result<Option<String>, GitError>;

    /// Read a file as it exists at the working-tree tip of `branch`.
    /// Convenience for "what's in the test file as of the merge candidate".
    fn read_at_branch(&self, branch: &str, path: &Path) -> Result<Option<String>, GitError> {
        self.read_at_commit(branch, path)
    }
}

/// Shells out to the `git` CLI. Repository root is captured at
/// construction.
pub struct ShellGitInspector {
    repo: PathBuf,
}

impl ShellGitInspector {
    pub fn new<P: Into<PathBuf>>(repo: P) -> Self {
        Self { repo: repo.into() }
    }

    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.repo)
            .output()?;
        if !out.status.success() {
            return Err(GitError::Command(format!(
                "git {} -> {}: {}",
                args.join(" "),
                out.status,
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl GitInspector for ShellGitInspector {
    fn commits_between(&self, base: &str, branch: &str) -> Result<Vec<CommitInfo>, GitError> {
        // `git log --reverse --pretty=format:'%H%x00%s' base..branch`
        // %x00 = literal NUL separator so subjects with `|` survive.
        let range = format!("{}..{}", base, branch);
        let log = self.run(&[
            "log",
            "--reverse",
            "--pretty=format:%H%x00%s",
            &range,
        ])?;

        let mut commits = Vec::new();
        for line in log.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, '\0');
            let sha = parts
                .next()
                .ok_or_else(|| GitError::Parse(format!("missing sha: {line:?}")))?
                .trim()
                .to_string();
            let subject = parts.next().unwrap_or("").trim().to_string();

            // `git show --name-status --pretty=format: <sha>` lists touched files.
            let names = self.run(&[
                "show",
                "--name-status",
                "--pretty=format:",
                "-m",
                "--first-parent",
                &sha,
            ])?;
            let files = parse_name_status(&names);

            commits.push(CommitInfo { sha, subject, files });
        }
        Ok(commits)
    }

    fn read_at_commit(&self, sha: &str, path: &Path) -> Result<Option<String>, GitError> {
        // `git show sha:path` — non-zero exit means path doesn't exist.
        let out = Command::new("git")
            .args(["show", &format!("{}:{}", sha, path.display())])
            .current_dir(&self.repo)
            .output()?;
        if !out.status.success() {
            // stderr usually mentions "exists on disk, but not in" or
            // "does not exist" — treat as "missing at this commit".
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
    }
}

fn parse_name_status(out: &str) -> Vec<FileChange> {
    let mut files = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Format: `<status>\t<path>` or `R<score>\t<old>\t<new>`.
        let mut cols = line.split('\t');
        let status = match cols.next() {
            Some(s) => s,
            None => continue,
        };
        // For rename we take the new path (last column).
        let path = if status.starts_with('R') || status.starts_with('C') {
            // skip old path; new path is last
            cols.next();
            match cols.next() {
                Some(p) => PathBuf::from(p),
                None => continue,
            }
        } else {
            match cols.next() {
                Some(p) => PathBuf::from(p),
                None => continue,
            }
        };
        files.push(FileChange {
            path,
            kind: FileChangeKind::from_status(status),
        });
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_name_status_basic() {
        let out = "A\tcrates/foo/src/lib.rs\nM\tcrates/foo/Cargo.toml\nD\tdocs/old.md\n";
        let files = parse_name_status(out);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, PathBuf::from("crates/foo/src/lib.rs"));
        assert_eq!(files[0].kind, FileChangeKind::Added);
        assert_eq!(files[1].kind, FileChangeKind::Modified);
        assert_eq!(files[2].kind, FileChangeKind::Deleted);
    }

    #[test]
    fn parse_name_status_rename() {
        let out = "R100\tdocs/old.md\tdocs/new.md\n";
        let files = parse_name_status(out);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("docs/new.md"));
        assert_eq!(files[0].kind, FileChangeKind::Renamed);
    }

    #[test]
    fn parse_name_status_empty_lines_skipped() {
        let out = "\nA\tcrates/foo/src/lib.rs\n\n\nM\tREADME.md\n";
        let files = parse_name_status(out);
        assert_eq!(files.len(), 2);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Git-history walker — walk every commit reachable from HEAD, run the
//! detector over each commit's diff, and tag findings with commit
//! metadata (sha, author, email, date, message).
//!
//! Mirrors `detect/git.go` upstream (`v8.29.1`). Implementation uses
//! `git2` (libgit2 bindings) so the walker is sovereign and doesn't shell
//! out to `git`.
//!
//! Out-of-scope this MVP:
//! - `--log-opts` arbitrary git rev-list arguments
//! - Submodule recursion
//! - Bare-repo scans (relies on a working tree on disk)
//! - Baseline diff (`--baseline-path`)

use std::path::Path;

use git2::{Commit, DiffOptions, Repository};
use thiserror::Error;

use crate::detect::Detector;
use crate::finding::Finding;

#[derive(Debug, Error)]
pub enum GitWalkError {
    #[error("git2 error: {0}")]
    Git(#[from] git2::Error),
}

/// Walk every commit on the current branch (HEAD ancestry), scan each
/// patch's added lines, and emit findings stamped with commit metadata.
///
/// `max_commits = None` walks the full history; `Some(n)` caps to the
/// most recent `n` commits (most-recent-first), matching the upstream
/// `--depth` flag.
pub fn scan_repo_history(
    repo_path: &Path,
    detector: &Detector,
    max_commits: Option<usize>,
) -> Result<Vec<Finding>, GitWalkError> {
    let repo = Repository::open(repo_path)?;
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    revwalk.set_sorting(git2::Sort::TIME)?;

    let mut all = Vec::new();
    let iter: Box<dyn Iterator<Item = _>> = match max_commits {
        Some(n) => Box::new(revwalk.take(n)),
        None => Box::new(revwalk),
    };
    for oid_res in iter {
        let oid = oid_res?;
        let commit = repo.find_commit(oid)?;
        let findings = scan_commit(&repo, &commit, detector)?;
        all.extend(findings);
    }
    Ok(all)
}

fn scan_commit(
    repo: &Repository,
    commit: &Commit,
    detector: &Detector,
) -> Result<Vec<Finding>, GitWalkError> {
    let parent = if commit.parent_count() > 0 {
        Some(commit.parent(0)?)
    } else {
        None
    };
    let parent_tree = parent.as_ref().map(|c| c.tree()).transpose()?;
    let tree = commit.tree()?;

    let mut opts = DiffOptions::new();
    opts.context_lines(0).interhunk_lines(0);

    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;

    let sha = commit.id().to_string();
    let author = commit.author();
    let author_name = author.name().unwrap_or("").to_string();
    let author_email = author.email().unwrap_or("").to_string();
    let date = format_unix_iso8601(commit.time().seconds());
    let message = commit.message().unwrap_or("").to_string();

    let mut findings = Vec::new();
    diff.foreach(
        &mut |_delta, _progress| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            if line.origin() != '+' {
                return true;
            }
            let Some(path) = delta.new_file().path() else {
                return true;
            };
            let path_str = path.display().to_string();
            let Ok(text) = std::str::from_utf8(line.content()) else {
                return true;
            };
            let base_line = line.new_lineno().unwrap_or(0) as usize;
            // Detector wants a "file" with line indices starting at 1.
            // We feed it one logical line at a time, then patch the
            // start/end_line to the real new_lineno from the hunk.
            let mut new_findings = detector.scan_str(&path_str, text.trim_end_matches('\n'));
            for f in &mut new_findings {
                f.start_line = base_line;
                f.end_line = base_line;
                f.commit = sha.clone();
                f.author = author_name.clone();
                f.email = author_email.clone();
                f.date = date.clone();
                f.message = message.clone();
                f.fingerprint = f.compute_fingerprint();
            }
            findings.extend(new_findings);
            true
        }),
    )?;
    Ok(findings)
}

/// Format a Unix timestamp as ISO-8601 UTC ("2026-05-19T11:24:00Z").
/// Hand-rolled to avoid pulling a date crate; matches upstream
/// `commit.Author.When.UTC().Format(time.RFC3339)`.
fn format_unix_iso8601(secs: i64) -> String {
    // Days since 1970-01-01 (Thursday).
    let mut days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let hour = rem / 3600;
    let minute = (rem / 60) % 60;
    let second = rem % 60;

    let mut year: i64 = 1970;
    loop {
        let leap = is_leap(year);
        let ydays = if leap { 366 } else { 365 };
        if days < ydays {
            break;
        }
        days -= ydays;
        year += 1;
    }
    let mlens = month_lengths(is_leap(year));
    let mut month = 1usize;
    let mut d = days;
    for (i, &ml) in mlens.iter().enumerate() {
        if d < ml as i64 {
            month = i + 1;
            break;
        }
        d -= ml as i64;
    }
    let day = (d + 1) as u32;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn month_lengths(leap: bool) -> [u32; 12] {
    [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;

    fn init_repo(path: &Path) -> Repository {
        let repo = Repository::init(path).unwrap();
        // Identity is needed to create commits.
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "test").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        repo
    }

    fn commit_file(repo: &Repository, path: &Path, name: &str, body: &str, msg: &str) {
        std::fs::write(path.join(name), body).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(name)).unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = Signature::now("test", "t@example.com").unwrap();
        let parent = repo.head().ok().and_then(|h| h.target()).and_then(|oid| repo.find_commit(oid).ok());
        let parents: Vec<&Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
            .unwrap();
    }

    #[test]
    fn iso8601_formats_known_epoch() {
        assert_eq!(format_unix_iso8601(0), "1970-01-01T00:00:00Z");
        // 2024-01-01T00:00:00Z = 1704067200
        assert_eq!(format_unix_iso8601(1_704_067_200), "2024-01-01T00:00:00Z");
        // 2024-02-29T12:34:56Z = leap day → 1709210096
        assert_eq!(format_unix_iso8601(1_709_210_096), "2024-02-29T12:34:56Z");
    }

    #[test]
    fn scan_repo_history_finds_secret_introduced_in_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo(tmp.path());
        commit_file(&repo, tmp.path(), "ok.txt", "no secrets here\n", "init");
        commit_file(
            &repo,
            tmp.path(),
            "leak.txt",
            "leak = AKIAIOSFODNN7EXAMPLE\n",
            "oops",
        );
        let d = Detector::with_builtins();
        let findings = scan_repo_history(tmp.path(), &d, None).unwrap();
        assert!(
            findings.iter().any(|f| f.rule_id == "aws-access-token"),
            "expected an aws finding in commit history"
        );
        let aws = findings
            .iter()
            .find(|f| f.rule_id == "aws-access-token")
            .unwrap();
        assert!(!aws.commit.is_empty());
        assert_eq!(aws.author, "test");
        assert_eq!(aws.email, "t@example.com");
        assert!(aws.message.contains("oops"));
    }

    #[test]
    fn max_commits_caps_walk_depth() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo(tmp.path());
        commit_file(&repo, tmp.path(), "a.txt", "AKIAIOSFODNN7EXAMPLE\n", "c1");
        commit_file(&repo, tmp.path(), "b.txt", "AKIAIOSFODNN7EXAMPLE\n", "c2");
        commit_file(&repo, tmp.path(), "c.txt", "AKIAIOSFODNN7EXAMPLE\n", "c3");
        let d = Detector::with_builtins();
        let limited = scan_repo_history(tmp.path(), &d, Some(1)).unwrap();
        // With depth=1 only the most-recent commit's diff is scanned.
        assert_eq!(limited.len(), 1);
    }
}

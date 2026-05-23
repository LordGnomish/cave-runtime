// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Git source — port of `pkg/sources/git/git.go`. Walks the working tree
//! or the full revision graph; optional `since` / `until` time bounds.

use crate::chunker::Chunker;
use crate::error::{Error, Result};
use crate::models::{Chunk, SourceKind, SourceMetadata};
use crate::sources::Source;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct GitOptions {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub branches: Vec<String>,
    pub bare: bool,
}

pub struct GitSource {
    pub repo: PathBuf,
    pub options: GitOptions,
    pub chunker: Chunker,
}

impl GitSource {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            options: GitOptions::default(),
            chunker: Chunker::default(),
        }
    }
}

impl Source for GitSource {
    fn name(&self) -> &str {
        "git"
    }
    fn chunks(&self) -> Result<Vec<Chunk>> {
        let repo = git2::Repository::open(&self.repo).map_err(|e| Error::Git(e.to_string()))?;
        let mut walk = repo.revwalk().map_err(|e| Error::Git(e.to_string()))?;
        if self.options.branches.is_empty() {
            walk.push_head().map_err(|e| Error::Git(e.to_string()))?;
        } else {
            for b in &self.options.branches {
                let r = format!("refs/heads/{}", b);
                walk.push_ref(&r).map_err(|e| Error::Git(e.to_string()))?;
            }
        }
        let mut out = Vec::new();
        for oid in walk {
            let oid = oid.map_err(|e| Error::Git(e.to_string()))?;
            let commit = repo
                .find_commit(oid)
                .map_err(|e| Error::Git(e.to_string()))?;
            let ts = DateTime::<Utc>::from_timestamp(commit.time().seconds(), 0).unwrap_or_default();
            if let Some(since) = &self.options.since
                && ts < *since
            {
                continue;
            }
            if let Some(until) = &self.options.until
                && ts > *until
            {
                continue;
            }
            let tree = commit.tree().map_err(|e| Error::Git(e.to_string()))?;
            let mut walker: Vec<(PathBuf, git2::Oid)> = Vec::new();
            tree.walk(git2::TreeWalkMode::PreOrder, |dir, e| {
                if e.kind() == Some(git2::ObjectType::Blob) {
                    let p = PathBuf::from(dir).join(e.name().unwrap_or_default());
                    walker.push((p, e.id()));
                }
                git2::TreeWalkResult::Ok
            })
            .map_err(|e| Error::Git(e.to_string()))?;
            for (path, blob_oid) in walker {
                let blob = repo
                    .find_blob(blob_oid)
                    .map_err(|e| Error::Git(e.to_string()))?;
                for cb in self.chunker.chunk_bytes(blob.content()) {
                    let mut c = Chunk::new("git", &path.display().to_string(), cb.data);
                    c.source_metadata = SourceMetadata {
                        kind: SourceKind::Git,
                        repository: self.repo.to_str().map(String::from),
                        commit: Some(oid.to_string()),
                        commit_author: commit
                            .author()
                            .name()
                            .map(|s| s.to_string()),
                        file: Some(path.display().to_string()),
                        timestamp: Some(ts.to_rfc3339()),
                        ..Default::default()
                    };
                    out.push(c);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_repo_with_commit(content: &[u8]) -> TempDir {
        let td = TempDir::new().unwrap();
        let repo = git2::Repository::init(td.path()).unwrap();
        fs::write(td.path().join("a.txt"), content).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("a.txt")).unwrap();
        idx.write().unwrap();
        let tree_oid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = git2::Signature::now("cave", "cave@cave.dev").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        td
    }

    #[test]
    fn walks_single_commit() {
        let td = init_repo_with_commit(b"hello git");
        let s = GitSource::new(td.path());
        let c = s.chunks().unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].source_metadata.kind, SourceKind::Git);
        assert!(c[0].source_metadata.commit.is_some());
        assert_eq!(c[0].source_metadata.commit_author.as_deref(), Some("cave"));
    }

    #[test]
    fn since_filter_excludes_old_commits() {
        let td = init_repo_with_commit(b"old");
        let future = Utc::now() + chrono::Duration::hours(1);
        let mut s = GitSource::new(td.path());
        s.options.since = Some(future);
        assert!(s.chunks().unwrap().is_empty());
    }

    #[test]
    fn name_is_git() {
        let td = init_repo_with_commit(b"x");
        let s = GitSource::new(td.path());
        assert_eq!(s.name(), "git");
    }
}

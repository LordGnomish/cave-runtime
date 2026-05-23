// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Filesystem source — port of `pkg/sources/filesystem/filesystem.go`.
//! Walks a root path, applies include/exclude globs, chunks each file.

use crate::chunker::Chunker;
use crate::error::{Error, Result};
use crate::models::{Chunk, SourceKind, SourceMetadata};
use crate::sources::Source;
use std::path::{Path, PathBuf};

pub struct FilesystemSource {
    pub root: PathBuf,
    pub excludes: Vec<String>,
    pub chunker: Chunker,
}

impl FilesystemSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            excludes: default_excludes(),
            chunker: Chunker::default(),
        }
    }

    pub fn with_excludes(mut self, exc: Vec<String>) -> Self {
        self.excludes = exc;
        self
    }
}

pub fn default_excludes() -> Vec<String> {
    vec![
        ".git".into(),
        "node_modules".into(),
        "target".into(),
        ".venv".into(),
        "__pycache__".into(),
    ]
}

fn is_excluded(p: &Path, exc: &[String]) -> bool {
    p.components()
        .any(|c| exc.iter().any(|e| c.as_os_str() == e.as_str()))
}

impl Source for FilesystemSource {
    fn name(&self) -> &str {
        "filesystem"
    }
    fn chunks(&self) -> Result<Vec<Chunk>> {
        let mut out = Vec::new();
        walk_dir(&self.root, &self.excludes, &mut out, &self.chunker)?;
        Ok(out)
    }
}

fn walk_dir(
    p: &Path,
    excludes: &[String],
    out: &mut Vec<Chunk>,
    ch: &Chunker,
) -> Result<()> {
    let entries = std::fs::read_dir(p).map_err(|e| Error::Source(e.to_string()))?;
    for e in entries.flatten() {
        let path = e.path();
        if is_excluded(&path, excludes) {
            continue;
        }
        if path.is_dir() {
            walk_dir(&path, excludes, out, ch)?;
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let chunks = ch.chunk_bytes(&data);
        for cb in chunks {
            let mut c =
                Chunk::new("filesystem", &path.display().to_string(), cb.data);
            c.source_metadata = SourceMetadata {
                kind: SourceKind::Filesystem,
                file: Some(path.display().to_string()),
                ..Default::default()
            };
            out.push(c);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn walks_temp_dir() {
        let td = TempDir::new().unwrap();
        fs::write(td.path().join("a.txt"), b"hello").unwrap();
        fs::write(td.path().join("b.txt"), b"world").unwrap();
        let s = FilesystemSource::new(td.path());
        let c = s.chunks().unwrap();
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|x| x.source_metadata.kind == SourceKind::Filesystem));
    }

    #[test]
    fn excludes_node_modules() {
        let td = TempDir::new().unwrap();
        fs::create_dir(td.path().join("node_modules")).unwrap();
        fs::write(td.path().join("node_modules").join("a.js"), b"x").unwrap();
        fs::write(td.path().join("ok.txt"), b"y").unwrap();
        let s = FilesystemSource::new(td.path());
        let c = s.chunks().unwrap();
        assert_eq!(c.len(), 1);
        assert!(c[0].source_metadata.file.as_deref().unwrap().contains("ok.txt"));
    }

    #[test]
    fn recursive_walk() {
        let td = TempDir::new().unwrap();
        fs::create_dir(td.path().join("nested")).unwrap();
        fs::write(td.path().join("nested").join("a.txt"), b"a").unwrap();
        let s = FilesystemSource::new(td.path());
        assert_eq!(s.chunks().unwrap().len(), 1);
    }

    #[test]
    fn empty_dir_yields_no_chunks() {
        let td = TempDir::new().unwrap();
        let s = FilesystemSource::new(td.path());
        assert!(s.chunks().unwrap().is_empty());
    }
}

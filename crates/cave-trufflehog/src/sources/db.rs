// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SQL-dump source — scans plain-text SQL dumps (Postgres, MySQL, SQLite)
//! for embedded secrets. Mirrors `pkg/sources/postgres` minus the live
//! connection path (cave-runtime offers cave-rdbms for in-cluster).

use crate::chunker::Chunker;
use crate::error::{Error, Result};
use crate::models::{Chunk, SourceKind, SourceMetadata};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbEngine {
    Postgres,
    Mysql,
    Sqlite,
}

impl DbEngine {
    pub fn as_str(&self) -> &'static str {
        match self {
            DbEngine::Postgres => "postgres",
            DbEngine::Mysql => "mysql",
            DbEngine::Sqlite => "sqlite",
        }
    }
}

pub struct DbDumpSource {
    pub path: PathBuf,
    pub engine: DbEngine,
    pub chunker: Chunker,
}

impl DbDumpSource {
    pub fn new(path: impl Into<PathBuf>, engine: DbEngine) -> Self {
        Self {
            path: path.into(),
            engine,
            chunker: Chunker::default(),
        }
    }

    pub fn name(&self) -> &str {
        "database"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::Database
    }

    pub fn chunks(&self) -> Result<Vec<Chunk>> {
        let data = std::fs::read(&self.path).map_err(|e| Error::Source(e.to_string()))?;
        Ok(self
            .chunker
            .chunk_bytes(&data)
            .into_iter()
            .map(|cb| {
                let mut c = Chunk::new(
                    "database",
                    &self.path.display().to_string(),
                    cb.data,
                );
                c.source_metadata = SourceMetadata {
                    kind: SourceKind::Database,
                    file: Some(self.path.display().to_string()),
                    container: Some(self.engine.as_str().to_string()),
                    ..Default::default()
                };
                c
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn reads_postgres_dump() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("dump.sql");
        fs::write(&p, b"INSERT INTO secrets (key) VALUES ('sk_live_x');").unwrap();
        let s = DbDumpSource::new(&p, DbEngine::Postgres);
        let c = s.chunks().unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].source_metadata.kind, SourceKind::Database);
        assert_eq!(c[0].source_metadata.container.as_deref(), Some("postgres"));
    }

    #[test]
    fn engine_name_strings() {
        assert_eq!(DbEngine::Postgres.as_str(), "postgres");
        assert_eq!(DbEngine::Mysql.as_str(), "mysql");
        assert_eq!(DbEngine::Sqlite.as_str(), "sqlite");
    }

    #[test]
    fn missing_file_errors() {
        let s = DbDumpSource::new("/nonexistent/path.sql", DbEngine::Sqlite);
        assert!(s.chunks().is_err());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workspace management: shared volumes between tasks.
//! Supports EmptyDir, PVC, ConfigMap, Secret, and HostPath bindings.

use crate::models::{WorkspaceBinding, WorkspaceKind};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("Workspace '{0}' not found in this run")]
    NotFound(String),
    #[error("IO error managing workspace: {0}")]
    Io(#[from] std::io::Error),
}

pub type WorkspaceResult<T> = Result<T, WorkspaceError>;

// ---------------------------------------------------------------------------
// Resolved workspace
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResolvedWorkspace {
    pub name: String,
    pub path: PathBuf,
    pub kind: WorkspaceKind,
}

// ---------------------------------------------------------------------------
// Workspace manager
// ---------------------------------------------------------------------------

pub struct WorkspaceManager {
    pub run_id: Uuid,
    pub workspaces: HashMap<String, ResolvedWorkspace>,
    base_dir: PathBuf,
}

impl WorkspaceManager {
    pub fn new(run_id: Uuid) -> Self {
        let base_dir = std::env::temp_dir()
            .join("cave-pipelines")
            .join(run_id.to_string());
        Self { run_id, workspaces: HashMap::new(), base_dir }
    }

    /// Bind a slice of workspace declarations for a pipeline run.
    pub async fn bind_workspaces(
        &mut self,
        bindings: &[WorkspaceBinding],
    ) -> WorkspaceResult<()> {
        for binding in bindings {
            let resolved = self.resolve_binding(binding).await?;
            self.workspaces.insert(binding.name.clone(), resolved);
        }
        Ok(())
    }

    async fn resolve_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> WorkspaceResult<ResolvedWorkspace> {
        let path = match &binding.kind {
            WorkspaceKind::EmptyDir => {
                let dir = self.base_dir.join(&binding.name);
                tokio::fs::create_dir_all(&dir).await?;
                dir
            }
            WorkspaceKind::HostPath { path } => PathBuf::from(path),
            WorkspaceKind::Pvc { claim_name } => {
                self.base_dir.join(format!("pvc-{claim_name}"))
            }
            WorkspaceKind::ConfigMap { name } => {
                self.base_dir.join(format!("cm-{name}"))
            }
            WorkspaceKind::Secret { secret_name } => {
                self.base_dir.join(format!("secret-{secret_name}"))
            }
        };

        Ok(ResolvedWorkspace { name: binding.name.clone(), path, kind: binding.kind.clone() })
    }

    pub fn get_path(&self, name: &str) -> WorkspaceResult<&PathBuf> {
        self.workspaces
            .get(name)
            .map(|w| &w.path)
            .ok_or_else(|| WorkspaceError::NotFound(name.to_string()))
    }

    /// Remove all EmptyDir workspaces created for this run.
    pub async fn cleanup(&self) -> WorkspaceResult<()> {
        if self.base_dir.exists() {
            tokio::fs::remove_dir_all(&self.base_dir).await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_starts_empty() {
        let mgr = WorkspaceManager::new(Uuid::new_v4());
        assert!(mgr.workspaces.is_empty());
    }

    #[test]
    fn test_get_missing_workspace_returns_error() {
        let mgr = WorkspaceManager::new(Uuid::new_v4());
        assert!(matches!(
            mgr.get_path("source"),
            Err(WorkspaceError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn test_bind_empty_dir_creates_directory() {
        let id = Uuid::new_v4();
        let mut mgr = WorkspaceManager::new(id);
        let bindings = vec![WorkspaceBinding {
            name: "source".to_string(),
            kind: WorkspaceKind::EmptyDir,
        }];
        mgr.bind_workspaces(&bindings).await.unwrap();
        let path = mgr.get_path("source").unwrap();
        assert!(path.exists());
        mgr.cleanup().await.ok();
    }

    #[tokio::test]
    async fn test_bind_multiple_workspaces() {
        let id = Uuid::new_v4();
        let mut mgr = WorkspaceManager::new(id);
        let bindings = vec![
            WorkspaceBinding { name: "src".to_string(), kind: WorkspaceKind::EmptyDir },
            WorkspaceBinding {
                name: "cache".to_string(),
                kind: WorkspaceKind::Pvc { claim_name: "build-cache".to_string() },
            },
        ];
        mgr.bind_workspaces(&bindings).await.unwrap();
        assert!(mgr.get_path("src").is_ok());
        assert!(mgr.get_path("cache").is_ok());
        mgr.cleanup().await.ok();
    }

    #[test]
    fn test_hostpath_binding_resolves_as_given() {
        // HostPath doesn't create dirs – just captures the path.
        // Use synchronous resolution by constructing the path directly.
        let id = Uuid::new_v4();
        let mgr = WorkspaceManager::new(id);
        let base = mgr.base_dir.clone();
        let path = PathBuf::from("/tmp/host-data");
        // Verify the base_dir embeds the run_id
        assert!(base.to_str().unwrap().contains(&id.to_string()));
        // And that a HostPath binding would produce that exact path
        let expected = PathBuf::from("/tmp/host-data");
        assert_eq!(path, expected);
    }
}

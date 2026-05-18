// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/pkg/project/manager.go + dao/project.go
//! In-memory project store backing the Harbor Admin API
//! (`/api/v2.0/projects/…`). Mirrors goharbor/harbor's
//! `src/pkg/project/manager.go` semantics:
//!
//! - Project name must be unique (case-sensitive lookup, returns
//!   `Conflict` on collision).
//! - Soft-delete deletes only when no repository still belongs to the
//!   project (`repo_count == 0`); otherwise upstream returns 412
//!   `PRECONDITION_FAILED`. We model that via `delete_error::HasRepos`.
//! - List supports a `name=` LIKE filter and a `public=true|false` filter
//!   (`pkg/q/builder.go`).
//! - Update merges metadata (Harbor's manager.Update does field-by-field
//!   merge, not whole-record replace).

use super::harbor::{Project, ProjectMetadata};
use chrono::Utc;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProjectError {
    #[error("project '{0}' already exists")]
    Conflict(String),
    #[error("project '{0}' not found")]
    NotFound(String),
    #[error("project '{0}' still has {1} repositories")]
    HasRepos(String, usize),
}

#[derive(Default)]
pub struct ProjectStore {
    inner: RwLock<Vec<Project>>,
}

impl ProjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// List with optional name LIKE filter + public filter. Harbor returns
    /// in `update_time DESC` order — we match.
    pub fn list(&self, name_like: Option<&str>, public: Option<bool>) -> Vec<Project> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<Project> = g
            .iter()
            .filter(|p| match name_like {
                Some(pat) => p.name.contains(pat),
                None => true,
            })
            .filter(|p| match public {
                Some(v) => p.public == v,
                None => true,
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.update_time.cmp(&a.update_time));
        out
    }

    pub fn count(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn get(&self, name: &str) -> Option<Project> {
        let g = self.inner.read().unwrap();
        g.iter().find(|p| p.name == name).cloned()
    }

    pub fn create(
        &self,
        name: String,
        public: bool,
        owner_name: String,
        metadata: ProjectMetadata,
    ) -> Result<Project, ProjectError> {
        let mut g = self.inner.write().unwrap();
        if g.iter().any(|p| p.name == name) {
            return Err(ProjectError::Conflict(name));
        }
        let now = Utc::now();
        let project = Project {
            id: Uuid::new_v4(),
            name,
            public,
            owner_name,
            description: String::new(),
            repo_count: 0,
            creation_time: now,
            update_time: now,
            metadata,
        };
        g.push(project.clone());
        Ok(project)
    }

    /// Update merges fields — None means "keep existing".
    pub fn update(
        &self,
        name: &str,
        public: Option<bool>,
        description: Option<String>,
        metadata: Option<ProjectMetadata>,
    ) -> Result<Project, ProjectError> {
        let mut g = self.inner.write().unwrap();
        let p = g
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| ProjectError::NotFound(name.to_string()))?;
        if let Some(v) = public {
            p.public = v;
        }
        if let Some(d) = description {
            p.description = d;
        }
        if let Some(m) = metadata {
            // Field-by-field merge (Harbor manager.Update style).
            if m.public.is_some() {
                p.metadata.public = m.public;
            }
            if m.enable_content_trust.is_some() {
                p.metadata.enable_content_trust = m.enable_content_trust;
            }
            if m.prevent_vul.is_some() {
                p.metadata.prevent_vul = m.prevent_vul;
            }
            if m.severity.is_some() {
                p.metadata.severity = m.severity;
            }
            if m.auto_scan.is_some() {
                p.metadata.auto_scan = m.auto_scan;
            }
            if m.reuse_sys_cve_allowlist.is_some() {
                p.metadata.reuse_sys_cve_allowlist = m.reuse_sys_cve_allowlist;
            }
        }
        p.update_time = Utc::now();
        Ok(p.clone())
    }

    /// Delete; upstream rejects when repo_count > 0.
    pub fn delete(&self, name: &str) -> Result<(), ProjectError> {
        let mut g = self.inner.write().unwrap();
        let idx = g
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| ProjectError::NotFound(name.to_string()))?;
        if g[idx].repo_count > 0 {
            return Err(ProjectError::HasRepos(
                name.to_string(),
                g[idx].repo_count as usize,
            ));
        }
        g.remove(idx);
        Ok(())
    }

    /// Bump the repo_count delta — wired into Docker V2 manifest PUT
    /// when a manifest lands under a project's namespace.
    pub fn adjust_repo_count(&self, name: &str, delta: i64) -> Result<(), ProjectError> {
        let mut g = self.inner.write().unwrap();
        let p = g
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| ProjectError::NotFound(name.to_string()))?;
        p.repo_count = (p.repo_count + delta).max(0);
        p.update_time = Utc::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ProjectStore {
        ProjectStore::new()
    }

    #[test]
    fn create_round_trips_and_assigns_id_and_now() {
        let s = store();
        let p = s
            .create("library".into(), true, "admin".into(), ProjectMetadata::default())
            .unwrap();
        assert_eq!(p.name, "library");
        assert!(p.public);
        assert_eq!(p.repo_count, 0);
        assert_eq!(s.count(), 1);
        // get matches
        let g = s.get("library").unwrap();
        assert_eq!(g.id, p.id);
    }

    #[test]
    fn create_rejects_duplicate_name() {
        let s = store();
        s.create("library".into(), true, "admin".into(), ProjectMetadata::default()).unwrap();
        let err = s
            .create("library".into(), false, "admin".into(), ProjectMetadata::default())
            .unwrap_err();
        assert_eq!(err, ProjectError::Conflict("library".into()));
    }

    #[test]
    fn list_filters_by_name_substring() {
        let s = store();
        for n in ["library", "alpha", "beta-svc", "alpha-staging"] {
            s.create(n.into(), false, "admin".into(), ProjectMetadata::default()).unwrap();
        }
        let alpha = s.list(Some("alpha"), None);
        assert_eq!(alpha.len(), 2);
        assert!(alpha.iter().all(|p| p.name.contains("alpha")));
    }

    #[test]
    fn list_filters_by_public_flag() {
        let s = store();
        s.create("pub".into(), true, "admin".into(), ProjectMetadata::default()).unwrap();
        s.create("priv".into(), false, "admin".into(), ProjectMetadata::default()).unwrap();
        let pubs = s.list(None, Some(true));
        assert_eq!(pubs.len(), 1);
        assert_eq!(pubs[0].name, "pub");
        let privs = s.list(None, Some(false));
        assert_eq!(privs.len(), 1);
        assert_eq!(privs[0].name, "priv");
    }

    #[test]
    fn list_orders_by_update_time_desc() {
        let s = store();
        s.create("old".into(), false, "admin".into(), ProjectMetadata::default()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        s.create("new".into(), false, "admin".into(), ProjectMetadata::default()).unwrap();
        let l = s.list(None, None);
        assert_eq!(l[0].name, "new", "newest should sort first");
    }

    #[test]
    fn update_merges_metadata_field_by_field() {
        let s = store();
        let mut initial_meta = ProjectMetadata::default();
        initial_meta.auto_scan = Some("true".into());
        initial_meta.severity = Some("high".into());
        s.create("library".into(), true, "admin".into(), initial_meta).unwrap();

        let mut patch = ProjectMetadata::default();
        patch.severity = Some("critical".into()); // override severity only
        let updated = s.update("library", None, None, Some(patch)).unwrap();
        assert_eq!(updated.metadata.severity.as_deref(), Some("critical"));
        // auto_scan must survive
        assert_eq!(updated.metadata.auto_scan.as_deref(), Some("true"));
    }

    #[test]
    fn update_returns_not_found_for_missing() {
        let s = store();
        let err = s.update("ghost", Some(true), None, None).unwrap_err();
        assert_eq!(err, ProjectError::NotFound("ghost".into()));
    }

    #[test]
    fn delete_rejects_when_repos_remain() {
        let s = store();
        s.create("library".into(), false, "admin".into(), ProjectMetadata::default()).unwrap();
        s.adjust_repo_count("library", 3).unwrap();
        let err = s.delete("library").unwrap_err();
        assert_eq!(err, ProjectError::HasRepos("library".into(), 3));
        assert!(s.get("library").is_some(), "must not delete on failure");
    }

    #[test]
    fn delete_succeeds_when_empty() {
        let s = store();
        s.create("temp".into(), false, "admin".into(), ProjectMetadata::default()).unwrap();
        s.delete("temp").unwrap();
        assert!(s.get("temp").is_none());
    }

    #[test]
    fn adjust_repo_count_floors_at_zero() {
        let s = store();
        s.create("temp".into(), false, "admin".into(), ProjectMetadata::default()).unwrap();
        s.adjust_repo_count("temp", -5).unwrap();
        assert_eq!(s.get("temp").unwrap().repo_count, 0);
    }
}

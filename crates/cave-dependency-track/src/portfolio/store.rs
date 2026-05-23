// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `PortfolioStore` — in-memory project + component registry.

use crate::error::{Error, Result};
use crate::models::{Classifier, Component, Project};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

use super::tags::normalize_tag;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProjectUpdate {
    pub name: Option<String>,
    pub version: Option<Option<String>>,
    pub classifier: Option<Classifier>,
    pub description: Option<Option<String>>,
    pub purl: Option<Option<String>>,
    pub cpe: Option<Option<String>>,
    pub active: Option<bool>,
    pub parent: Option<Option<Uuid>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Default)]
pub struct PortfolioStore {
    projects: RwLock<HashMap<Uuid, Project>>,
    components: RwLock<HashMap<Uuid, Component>>,
    by_project: RwLock<HashMap<Uuid, Vec<Uuid>>>,
}

impl PortfolioStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.projects.read().unwrap().len()
    }

    pub fn component_count(&self) -> usize {
        self.components.read().unwrap().len()
    }

    pub fn insert(&self, mut p: Project) -> Result<Project> {
        let mut guard = self.projects.write().unwrap();
        if guard.values().any(|q| q.name == p.name && q.version == p.version) {
            return Err(Error::Conflict(format!(
                "project name={} version={:?} already exists",
                p.name, p.version
            )));
        }
        if let Some(pp) = p.parent {
            if !guard.contains_key(&pp) {
                return Err(Error::NotFound(format!("parent project {}", pp)));
            }
            if pp == p.uuid {
                return Err(Error::Invalid("project cannot be its own parent".into()));
            }
        }
        p.tags = p
            .tags
            .into_iter()
            .map(|t| normalize_tag(&t))
            .filter(|t| !t.is_empty())
            .collect();
        p.created = Utc::now();
        guard.insert(p.uuid, p.clone());
        Ok(p)
    }

    pub fn get(&self, uuid: Uuid) -> Result<Project> {
        self.projects
            .read()
            .unwrap()
            .get(&uuid)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("project {}", uuid)))
    }

    pub fn list(&self) -> Vec<Project> {
        let mut v: Vec<_> = self.projects.read().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
        v
    }

    pub fn list_active(&self) -> Vec<Project> {
        self.list().into_iter().filter(|p| p.active).collect()
    }

    pub fn list_by_tag(&self, tag: &str) -> Vec<Project> {
        let want = normalize_tag(tag);
        self.list()
            .into_iter()
            .filter(|p| p.tags.iter().any(|t| t == &want))
            .collect()
    }

    pub fn update(&self, uuid: Uuid, upd: ProjectUpdate) -> Result<Project> {
        let mut guard = self.projects.write().unwrap();
        let p = guard
            .get_mut(&uuid)
            .ok_or_else(|| Error::NotFound(format!("project {}", uuid)))?;
        if let Some(v) = upd.name {
            p.name = v;
        }
        if let Some(v) = upd.version {
            p.version = v;
        }
        if let Some(v) = upd.classifier {
            p.classifier = v;
        }
        if let Some(v) = upd.description {
            p.description = v;
        }
        if let Some(v) = upd.purl {
            p.purl = v;
        }
        if let Some(v) = upd.cpe {
            p.cpe = v;
        }
        if let Some(v) = upd.active {
            p.active = v;
        }
        if let Some(v) = upd.parent {
            if let Some(pp) = v {
                if pp == uuid {
                    return Err(Error::Invalid("project cannot be its own parent".into()));
                }
            }
            p.parent = v;
        }
        if let Some(v) = upd.tags {
            p.tags = v
                .into_iter()
                .map(|t| normalize_tag(&t))
                .filter(|t| !t.is_empty())
                .collect();
        }
        Ok(p.clone())
    }

    pub fn delete(&self, uuid: Uuid) -> Result<()> {
        let mut guard = self.projects.write().unwrap();
        if guard.remove(&uuid).is_none() {
            return Err(Error::NotFound(format!("project {}", uuid)));
        }
        // Cascade — remove components.
        let mut comps = self.components.write().unwrap();
        let mut idx = self.by_project.write().unwrap();
        if let Some(ids) = idx.remove(&uuid) {
            for id in ids {
                comps.remove(&id);
            }
        }
        Ok(())
    }

    pub fn add_component(&self, c: Component) -> Result<Component> {
        if !self.projects.read().unwrap().contains_key(&c.project) {
            return Err(Error::NotFound(format!("project {}", c.project)));
        }
        let mut comps = self.components.write().unwrap();
        comps.insert(c.uuid, c.clone());
        self.by_project
            .write()
            .unwrap()
            .entry(c.project)
            .or_default()
            .push(c.uuid);
        Ok(c)
    }

    pub fn components_for(&self, project: Uuid) -> Vec<Component> {
        let comps = self.components.read().unwrap();
        let idx = self.by_project.read().unwrap();
        idx.get(&project)
            .map(|ids| ids.iter().filter_map(|i| comps.get(i).cloned()).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_proj(n: &str) -> Project {
        Project::new(n, Classifier::Application)
    }

    #[test]
    fn insert_then_get() {
        let s = PortfolioStore::new();
        let p = s.insert(new_proj("cave")).unwrap();
        let back = s.get(p.uuid).unwrap();
        assert_eq!(back.name, "cave");
    }

    #[test]
    fn insert_duplicate_name_version_fails() {
        let s = PortfolioStore::new();
        let _ = s.insert(new_proj("cave")).unwrap();
        let err = s.insert(new_proj("cave")).unwrap_err();
        assert!(matches!(err, Error::Conflict(_)));
    }

    #[test]
    fn parent_must_exist() {
        let s = PortfolioStore::new();
        let mut p = new_proj("child");
        p.parent = Some(Uuid::new_v4());
        let err = s.insert(p).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn list_returns_sorted_by_name() {
        let s = PortfolioStore::new();
        s.insert(new_proj("zeta")).unwrap();
        s.insert(new_proj("alpha")).unwrap();
        let l = s.list();
        assert_eq!(l[0].name, "alpha");
        assert_eq!(l[1].name, "zeta");
    }

    #[test]
    fn update_tags_normalises_and_dedupes_empty() {
        let s = PortfolioStore::new();
        let p = s.insert(new_proj("cave")).unwrap();
        let upd = ProjectUpdate {
            tags: Some(vec!["Prod".into(), "  ".into(), " sov ".into()]),
            ..Default::default()
        };
        let back = s.update(p.uuid, upd).unwrap();
        assert_eq!(back.tags, vec!["prod", "sov"]);
    }

    #[test]
    fn delete_cascades_components() {
        let s = PortfolioStore::new();
        let p = s.insert(new_proj("cave")).unwrap();
        s.add_component(Component::new(p.uuid, "lib1")).unwrap();
        s.add_component(Component::new(p.uuid, "lib2")).unwrap();
        assert_eq!(s.component_count(), 2);
        s.delete(p.uuid).unwrap();
        assert_eq!(s.component_count(), 0);
    }

    #[test]
    fn add_component_requires_project() {
        let s = PortfolioStore::new();
        let c = Component::new(Uuid::new_v4(), "lib");
        assert!(matches!(s.add_component(c), Err(Error::NotFound(_))));
    }

    #[test]
    fn list_by_tag_filters() {
        let s = PortfolioStore::new();
        let mut p1 = new_proj("a");
        p1.tags = vec!["prod".into()];
        let mut p2 = new_proj("b");
        p2.tags = vec!["dev".into()];
        s.insert(p1).unwrap();
        s.insert(p2).unwrap();
        assert_eq!(s.list_by_tag("PROD").len(), 1);
        assert_eq!(s.list_by_tag("dev").len(), 1);
        assert_eq!(s.list_by_tag("none").len(), 0);
    }

    #[test]
    fn update_cannot_self_parent() {
        let s = PortfolioStore::new();
        let p = s.insert(new_proj("cave")).unwrap();
        let upd = ProjectUpdate {
            parent: Some(Some(p.uuid)),
            ..Default::default()
        };
        assert!(matches!(s.update(p.uuid, upd), Err(Error::Invalid(_))));
    }

    #[test]
    fn deactivate_via_update() {
        let s = PortfolioStore::new();
        let p = s.insert(new_proj("cave")).unwrap();
        let upd = ProjectUpdate {
            active: Some(false),
            ..Default::default()
        };
        let back = s.update(p.uuid, upd).unwrap();
        assert!(!back.active);
        assert_eq!(s.list_active().len(), 0);
    }
}

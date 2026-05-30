// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composition store with revision history (preserved from pre-port scaffold).

use crate::composition::revision_gc::RevisionGarbageCollector;
use crate::error::{CrossplaneError, CrossplaneResult};
use crate::models::{Composition, CompositionStatus, CreateCompositionRequest};
use chrono::Utc;
use dashmap::DashMap;
use std::collections::VecDeque;
use uuid::Uuid;

pub struct CompositionStore {
    compositions: DashMap<String, Composition>,
    /// `{api_version}/{kind}` → Vec of composition names
    type_index: DashMap<String, Vec<String>>,
    /// name → revision history (VecDeque of Composition snapshots)
    revision_history: DashMap<String, VecDeque<Composition>>,
}

impl CompositionStore {
    pub fn new() -> Self {
        Self {
            compositions: DashMap::new(),
            type_index: DashMap::new(),
            revision_history: DashMap::new(),
        }
    }

    pub fn create(&self, req: CreateCompositionRequest) -> CrossplaneResult<Composition> {
        if req.name.is_empty() {
            return Err(CrossplaneError::CompositionValidation(
                "name must not be empty".into(),
            ));
        }
        if self.compositions.contains_key(&req.name) {
            return Err(CrossplaneError::CompositionValidation(format!(
                "Composition already exists: {}",
                req.name
            )));
        }

        let type_key = format!(
            "{}/{}",
            req.composite_type_ref.api_version, req.composite_type_ref.kind
        );

        let composition = Composition {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            composite_type_ref: req.composite_type_ref,
            resources: req.resources,
            pipeline: req.pipeline,
            mode: req.mode,
            patch_sets: req.patch_sets,
            status: CompositionStatus::Available,
            revision: 1,
            created_at: Utc::now(),
        };

        self.type_index
            .entry(type_key)
            .or_default()
            .push(req.name.clone());

        let mut history = VecDeque::new();
        history.push_back(composition.clone());
        self.revision_history.insert(req.name.clone(), history);

        self.compositions.insert(req.name, composition.clone());
        Ok(composition)
    }

    pub fn get(&self, name: &str) -> CrossplaneResult<Composition> {
        self.compositions
            .get(name)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::CompositionNotFound(name.to_owned()))
    }

    pub fn list(&self) -> Vec<Composition> {
        self.compositions
            .iter()
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn list_for_type(&self, api_version: &str, kind: &str) -> Vec<Composition> {
        let type_key = format!("{}/{}", api_version, kind);
        let names: Vec<String> = self
            .type_index
            .get(&type_key)
            .map(|r| r.clone())
            .unwrap_or_default();
        names
            .iter()
            .filter_map(|n| self.compositions.get(n).map(|r| r.clone()))
            .collect()
    }

    pub fn delete(&self, name: &str) -> CrossplaneResult<()> {
        match self.compositions.remove(name) {
            Some((_, composition)) => {
                let type_key = format!(
                    "{}/{}",
                    composition.composite_type_ref.api_version, composition.composite_type_ref.kind
                );
                if let Some(mut names) = self.type_index.get_mut(&type_key) {
                    names.retain(|n| n != name);
                }
                self.revision_history.remove(name);
                Ok(())
            }
            None => Err(CrossplaneError::CompositionNotFound(name.to_owned())),
        }
    }

    pub fn get_revisions(&self, name: &str) -> CrossplaneResult<Vec<Composition>> {
        self.revision_history
            .get(name)
            .map(|r| r.iter().cloned().collect())
            .ok_or_else(|| CrossplaneError::CompositionNotFound(name.to_owned()))
    }

    /// Append a new revision snapshot. Retention is an explicit reconcile
    /// concern handled by [`gc_revisions`](Self::gc_revisions) — mirroring
    /// upstream, where CompositionRevisions are created on every spec change
    /// and garbage-collected separately by `revisionHistoryLimit`.
    pub fn push_revision(&self, name: &str, composition: Composition) {
        if let Some(mut history) = self.revision_history.get_mut(name) {
            history.push_back(composition);
        }
    }

    /// Garbage-collect old revisions per `revisionHistoryLimit`, preserving the
    /// current (highest-numbered) revision and the newest `limit` historical
    /// revisions. Returns the number of revisions collected.
    ///
    /// `None` → default limit (1); `Some(0)` → keep all (GC disabled);
    /// `Some(n)` → keep current + `n`. Upstream
    /// `internal/controller/pkg/manager/reconciler.go` GC block.
    pub fn gc_revisions(&self, name: &str, limit: Option<i64>) -> CrossplaneResult<usize> {
        let mut history = self
            .revision_history
            .get_mut(name)
            .ok_or_else(|| CrossplaneError::CompositionNotFound(name.to_owned()))?;
        let revisions: Vec<u32> = history.iter().map(|c| c.revision).collect();
        let current = revisions.iter().copied().max().unwrap_or(0);
        let to_collect: std::collections::HashSet<u32> =
            RevisionGarbageCollector::plan(&revisions, current, limit)
                .into_iter()
                .collect();
        if to_collect.is_empty() {
            return Ok(0);
        }
        let before = history.len();
        history.retain(|c| !to_collect.contains(&c.revision));
        Ok(before - history.len())
    }

    /// Number of compositions currently registered.
    pub fn len(&self) -> usize {
        self.compositions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.compositions.is_empty()
    }
}

impl Default for CompositionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CompositionMode, TypeRef};

    fn req(name: &str) -> CreateCompositionRequest {
        CreateCompositionRequest {
            name: name.into(),
            composite_type_ref: TypeRef {
                api_version: "ex.cave.io/v1".into(),
                kind: "XDb".into(),
            },
            resources: vec![],
            pipeline: vec![],
            mode: CompositionMode::Pipeline,
            patch_sets: vec![],
        }
    }

    #[test]
    fn create_then_get() {
        let s = CompositionStore::new();
        let _ = s.create(req("c1")).unwrap();
        assert_eq!(s.get("c1").unwrap().name, "c1");
    }

    #[test]
    fn empty_name_rejected() {
        let s = CompositionStore::new();
        assert!(s.create(req("")).is_err());
    }

    #[test]
    fn duplicate_rejected() {
        let s = CompositionStore::new();
        s.create(req("dup")).unwrap();
        assert!(s.create(req("dup")).is_err());
    }

    #[test]
    fn list_for_type() {
        let s = CompositionStore::new();
        s.create(req("a")).unwrap();
        s.create(req("b")).unwrap();
        assert_eq!(s.list_for_type("ex.cave.io/v1", "XDb").len(), 2);
    }

    #[test]
    fn delete_removes() {
        let s = CompositionStore::new();
        s.create(req("c1")).unwrap();
        s.delete("c1").unwrap();
        assert!(s.get("c1").is_err());
    }

    #[test]
    fn revisions_initial() {
        let s = CompositionStore::new();
        s.create(req("c1")).unwrap();
        assert_eq!(s.get_revisions("c1").unwrap().len(), 1);
    }

    #[test]
    fn push_revision_appends_unbounded() {
        // Retention is now an explicit GC concern; push is append-only.
        let s = CompositionStore::new();
        let mut c = s.create(req("c1")).unwrap();
        for r in 2..=16u32 {
            c.revision = r;
            s.push_revision("c1", c.clone());
        }
        assert_eq!(s.get_revisions("c1").unwrap().len(), 16);
    }

    #[test]
    fn gc_revisions_honours_history_limit() {
        let s = CompositionStore::new();
        let mut c = s.create(req("c1")).unwrap();
        for r in 2..=12u32 {
            c.revision = r;
            s.push_revision("c1", c.clone());
        }
        let collected = s.gc_revisions("c1", Some(4)).unwrap();
        assert_eq!(collected, 7); // 12 - (current + 4)
        let nums: Vec<u32> = s
            .get_revisions("c1")
            .unwrap()
            .iter()
            .map(|r| r.revision)
            .collect();
        assert_eq!(nums, vec![8, 9, 10, 11, 12]);
    }
}

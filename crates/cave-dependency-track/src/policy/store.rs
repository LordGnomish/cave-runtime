// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `PolicyStore` — Policy CRUD + project assignment.

use super::engine::Policy;
use crate::error::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct PolicyStore {
    policies: RwLock<HashMap<Uuid, Policy>>,
    /// Policy → assigned projects (empty == global).
    assignments: RwLock<HashMap<Uuid, HashSet<Uuid>>>,
    /// `license_group_name → [SPDX-ids]`.
    license_groups: RwLock<HashMap<String, Vec<String>>>,
}

impl PolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.policies.read().unwrap().len()
    }

    pub fn put(&self, p: Policy) -> Policy {
        self.policies.write().unwrap().insert(p.uuid, p.clone());
        p
    }

    pub fn get(&self, uuid: Uuid) -> Result<Policy> {
        self.policies
            .read()
            .unwrap()
            .get(&uuid)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("policy {}", uuid)))
    }

    pub fn list(&self) -> Vec<Policy> {
        let mut v: Vec<_> = self.policies.read().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn delete(&self, uuid: Uuid) -> Result<()> {
        if self.policies.write().unwrap().remove(&uuid).is_none() {
            return Err(Error::NotFound(format!("policy {}", uuid)));
        }
        self.assignments.write().unwrap().remove(&uuid);
        Ok(())
    }

    pub fn assign(&self, policy: Uuid, project: Uuid) -> Result<()> {
        self.get(policy)?;
        self.assignments
            .write()
            .unwrap()
            .entry(policy)
            .or_default()
            .insert(project);
        Ok(())
    }

    pub fn unassign(&self, policy: Uuid, project: Uuid) -> Result<()> {
        let mut guard = self.assignments.write().unwrap();
        if let Some(set) = guard.get_mut(&policy) {
            set.remove(&project);
            if set.is_empty() {
                guard.remove(&policy);
            }
        }
        Ok(())
    }

    /// Returns all policies applicable to `project`: globally-assigned (no
    /// project list) policies plus those explicitly assigned.
    pub fn policies_for(&self, project: Uuid) -> Vec<Policy> {
        let assignments = self.assignments.read().unwrap();
        self.list()
            .into_iter()
            .filter(|p| match assignments.get(&p.uuid) {
                Some(set) if !set.is_empty() => set.contains(&project),
                _ => true,
            })
            .collect()
    }

    pub fn put_license_group(&self, name: impl Into<String>, members: Vec<String>) {
        self.license_groups
            .write()
            .unwrap()
            .insert(name.into(), members);
    }

    pub fn license_groups_snapshot(&self) -> HashMap<String, Vec<String>> {
        self.license_groups.read().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_roundtrip() {
        let s = PolicyStore::new();
        let p = s.put(Policy::new("strict"));
        assert_eq!(s.get(p.uuid).unwrap().name, "strict");
    }

    #[test]
    fn delete_removes_and_unassigns() {
        let s = PolicyStore::new();
        let p = s.put(Policy::new("p"));
        let proj = Uuid::new_v4();
        s.assign(p.uuid, proj).unwrap();
        s.delete(p.uuid).unwrap();
        assert!(s.get(p.uuid).is_err());
    }

    #[test]
    fn unassigned_policy_is_global() {
        let s = PolicyStore::new();
        s.put(Policy::new("global"));
        let proj = Uuid::new_v4();
        assert_eq!(s.policies_for(proj).len(), 1);
    }

    #[test]
    fn assigned_policy_only_for_target() {
        let s = PolicyStore::new();
        let p = s.put(Policy::new("scoped"));
        let proj_a = Uuid::new_v4();
        let proj_b = Uuid::new_v4();
        s.assign(p.uuid, proj_a).unwrap();
        assert_eq!(s.policies_for(proj_a).len(), 1);
        assert_eq!(s.policies_for(proj_b).len(), 0);
    }

    #[test]
    fn unassign_promotes_back_to_global() {
        let s = PolicyStore::new();
        let p = s.put(Policy::new("p"));
        let proj = Uuid::new_v4();
        s.assign(p.uuid, proj).unwrap();
        s.unassign(p.uuid, proj).unwrap();
        // Empty set is removed → policy becomes global.
        assert_eq!(s.policies_for(Uuid::new_v4()).len(), 1);
    }

    #[test]
    fn license_groups_snapshot() {
        let s = PolicyStore::new();
        s.put_license_group("copyleft", vec!["GPL-3.0".into(), "AGPL-3.0".into()]);
        let snap = s.license_groups_snapshot();
        assert_eq!(snap.get("copyleft").unwrap().len(), 2);
    }
}

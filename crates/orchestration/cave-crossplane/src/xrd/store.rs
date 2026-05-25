// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XRD (Composite Resource Definition) store (preserved from pre-port scaffold).

use crate::error::{CrossplaneError, CrossplaneResult};
use crate::models::{CreateXrdRequest, Xrd, XrdStatus};
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub struct XrdStore {
    xrds: DashMap<String, Xrd>,
    /// claim_kind → xrd key (`{group}/{kind}`)
    claim_index: DashMap<String, String>,
}

impl XrdStore {
    pub fn new() -> Self {
        Self {
            xrds: DashMap::new(),
            claim_index: DashMap::new(),
        }
    }

    pub fn create(&self, req: CreateXrdRequest) -> CrossplaneResult<Xrd> {
        if req.group.is_empty() {
            return Err(CrossplaneError::XrdValidation(
                "group must not be empty".into(),
            ));
        }
        if req.kind.is_empty() {
            return Err(CrossplaneError::XrdValidation(
                "kind must not be empty".into(),
            ));
        }

        let key = format!("{}/{}", req.group, req.kind);
        if self.xrds.contains_key(&key) {
            return Err(CrossplaneError::XrdValidation(format!(
                "XRD already exists: {}",
                key
            )));
        }

        let list_kind = format!("{}List", req.kind);
        let claim_list_kind = req.claim_kind.as_ref().map(|ck| format!("{}List", ck));

        let xrd = Xrd {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            group: req.group.clone(),
            kind: req.kind.clone(),
            list_kind,
            claim_kind: req.claim_kind.clone(),
            claim_list_kind,
            versions: req.versions,
            scope: req.scope,
            status: XrdStatus::Offering,
            created_at: Utc::now(),
        };

        if let Some(ref ck) = req.claim_kind {
            self.claim_index.insert(ck.clone(), key.clone());
        }

        self.xrds.insert(key, xrd.clone());
        Ok(xrd)
    }

    pub fn get(&self, group: &str, kind: &str) -> CrossplaneResult<Xrd> {
        let key = format!("{}/{}", group, kind);
        self.xrds
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::XrdNotFound(key))
    }

    pub fn get_by_name(&self, name: &str) -> CrossplaneResult<Xrd> {
        self.xrds
            .iter()
            .find(|r| r.value().name == name)
            .map(|r| r.value().clone())
            .ok_or_else(|| CrossplaneError::XrdNotFound(name.to_owned()))
    }

    pub fn list(&self) -> Vec<Xrd> {
        self.xrds.iter().map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, group: &str, kind: &str) -> CrossplaneResult<()> {
        let key = format!("{}/{}", group, kind);
        match self.xrds.remove(&key) {
            Some((_, xrd)) => {
                if let Some(ref ck) = xrd.claim_kind {
                    self.claim_index.remove(ck);
                }
                Ok(())
            }
            None => Err(CrossplaneError::XrdNotFound(key)),
        }
    }

    pub fn get_by_claim_kind(&self, claim_kind: &str) -> CrossplaneResult<Xrd> {
        let key = self
            .claim_index
            .get(claim_kind)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::XrdNotFound(claim_kind.to_owned()))?;
        self.xrds
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::XrdNotFound(key))
    }

    pub fn len(&self) -> usize {
        self.xrds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.xrds.is_empty()
    }
}

impl Default for XrdStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::XrdScope;

    fn req(group: &str, kind: &str, claim: Option<&str>) -> CreateXrdRequest {
        CreateXrdRequest {
            name: format!("{}.{}", kind.to_lowercase(), group),
            group: group.into(),
            kind: kind.into(),
            claim_kind: claim.map(String::from),
            scope: XrdScope::Cluster,
            versions: vec![],
        }
    }

    #[test]
    fn create_and_get() {
        let s = XrdStore::new();
        s.create(req("ex.cave.io", "XDb", None)).unwrap();
        assert!(s.get("ex.cave.io", "XDb").is_ok());
    }

    #[test]
    fn missing_group_rejected() {
        let s = XrdStore::new();
        assert!(s.create(req("", "Z", None)).is_err());
    }

    #[test]
    fn missing_kind_rejected() {
        let s = XrdStore::new();
        assert!(s.create(req("g", "", None)).is_err());
    }

    #[test]
    fn duplicate_rejected() {
        let s = XrdStore::new();
        s.create(req("g", "K", None)).unwrap();
        assert!(s.create(req("g", "K", None)).is_err());
    }

    #[test]
    fn claim_index_lookup() {
        let s = XrdStore::new();
        s.create(req("g", "K", Some("KC"))).unwrap();
        assert_eq!(s.get_by_claim_kind("KC").unwrap().kind, "K");
    }

    #[test]
    fn delete_removes() {
        let s = XrdStore::new();
        s.create(req("g", "K", Some("KC"))).unwrap();
        s.delete("g", "K").unwrap();
        assert!(s.get("g", "K").is_err());
        assert!(s.get_by_claim_kind("KC").is_err());
    }

    #[test]
    fn list_returns_all() {
        let s = XrdStore::new();
        s.create(req("g", "A", None)).unwrap();
        s.create(req("g", "B", None)).unwrap();
        assert_eq!(s.list().len(), 2);
    }
}

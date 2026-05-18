// SPDX-License-Identifier: AGPL-3.0-or-later
//! XRD (Composite Resource Definition) store.

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
}

impl Default for XrdStore {
    fn default() -> Self {
        Self::new()
    }
}

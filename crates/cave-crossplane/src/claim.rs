// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Claim and CompositeResource store.

use crate::engine::CompositionEngine;
use crate::error::{CrossplaneError, CrossplaneResult};
use crate::models::{
    Claim, ClaimRef, ClaimStatus, ClaimSyncStatus, CompositeResource, CompositeStatus, Composition,
    CreateClaimRequest, DeletionPolicy, Xrd,
};
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub struct ClaimStore {
    /// keyed by `{namespace}/{name}/{kind}`
    claims: DashMap<String, Claim>,
    /// keyed by `{kind}/{name}`
    composites: DashMap<String, CompositeResource>,
    claim_to_composite: DashMap<String, String>,
    composite_to_claim: DashMap<String, String>,
}

impl ClaimStore {
    pub fn new() -> Self {
        Self {
            claims: DashMap::new(),
            composites: DashMap::new(),
            claim_to_composite: DashMap::new(),
            composite_to_claim: DashMap::new(),
        }
    }

    fn claim_key(ns: &str, name: &str, kind: &str) -> String {
        format!("{}/{}/{}", ns, name, kind)
    }

    fn composite_key(kind: &str, name: &str) -> String {
        format!("{}/{}", kind, name)
    }

    pub fn create_claim(
        &self,
        req: CreateClaimRequest,
        xrd: &Xrd,
        composition: &Composition,
        engine: &CompositionEngine,
    ) -> CrossplaneResult<(Claim, CompositeResource)> {
        // Validate spec against XRD schema (check required fields in first referenceable version)
        if let Some(version) = xrd.versions.iter().find(|v| v.referenceable) {
            if let Some(schema) = &version.schema {
                for required_field in &schema.required {
                    if crate::engine::get_field_path(&req.spec, required_field).is_none() {
                        return Err(CrossplaneError::ClaimValidation(format!(
                            "required field '{}' missing from spec",
                            required_field
                        )));
                    }
                }
            }
        }

        // Render composition
        let rendered = engine.render(composition, &req.spec)?;

        let claim_key = Self::claim_key(&req.namespace, &req.name, &req.kind);
        let composite_name = format!("{}-{}", req.name, &Uuid::new_v4().to_string()[..8]);
        let composite_kind = xrd.kind.clone();
        let composite_key = Self::composite_key(&composite_kind, &composite_name);

        let claim = Claim {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            kind: req.kind.clone(),
            api_version: req.api_version.clone(),
            spec: req.spec.clone(),
            status: ClaimStatus::Waiting,
            composite_ref: Some(composite_name.clone()),
            sync_status: ClaimSyncStatus::Unknown,
            created_at: Utc::now(),
        };

        let composite = CompositeResource {
            id: Uuid::new_v4(),
            name: composite_name,
            namespace: None,
            kind: composite_kind,
            api_version: format!("{}/v1", xrd.group),
            spec: req.spec.clone(),
            status: CompositeStatus::Creating,
            composition_ref: Some(composition.name.clone()),
            claim_ref: Some(ClaimRef {
                namespace: req.namespace.clone(),
                name: req.name,
            }),
            synced_resources: rendered
                .iter()
                .enumerate()
                .map(|(i, _)| format!("resource-{}", i))
                .collect(),
            created_at: Utc::now(),
        };

        self.claims.insert(claim_key.clone(), claim.clone());
        self.composites
            .insert(composite_key.clone(), composite.clone());
        self.claim_to_composite
            .insert(claim_key.clone(), composite_key.clone());
        self.composite_to_claim.insert(composite_key, claim_key);

        Ok((claim, composite))
    }

    pub fn get_claim(&self, ns: &str, name: &str, kind: &str) -> CrossplaneResult<Claim> {
        let key = Self::claim_key(ns, name, kind);
        self.claims
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::ClaimNotFound(key))
    }

    pub fn list_claims_for_namespace(&self, ns: &str) -> Vec<Claim> {
        self.claims
            .iter()
            .filter(|r| r.value().namespace == ns)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_claim(
        &self,
        ns: &str,
        name: &str,
        kind: &str,
        deletion_policy: DeletionPolicy,
    ) -> CrossplaneResult<()> {
        let key = Self::claim_key(ns, name, kind);
        match self.claims.remove(&key) {
            Some(_) => {
                if let Some((_, composite_key)) = self.claim_to_composite.remove(&key) {
                    self.composite_to_claim.remove(&composite_key);
                    match deletion_policy {
                        DeletionPolicy::Delete => {
                            self.composites.remove(&composite_key);
                        }
                        DeletionPolicy::Orphan => {
                            // Leave composite in place
                        }
                    }
                }
                Ok(())
            }
            None => Err(CrossplaneError::ClaimNotFound(key)),
        }
    }

    pub fn sync_claim_from_composite(&self, claim_key: &str) -> CrossplaneResult<()> {
        let composite_key = self
            .claim_to_composite
            .get(claim_key)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::ClaimNotFound(claim_key.to_owned()))?;

        let composite_status = self
            .composites
            .get(&composite_key)
            .map(|r| r.status.clone())
            .ok_or_else(|| CrossplaneError::CompositeNotFound(composite_key.clone()))?;

        if let Some(mut claim) = self.claims.get_mut(claim_key) {
            claim.status = match composite_status {
                CompositeStatus::Ready => ClaimStatus::Ready,
                CompositeStatus::Creating => ClaimStatus::Waiting,
                CompositeStatus::Deleting => ClaimStatus::Deleting,
                CompositeStatus::Unready => ClaimStatus::Unready,
            };
            claim.sync_status = ClaimSyncStatus::Synced;
        }

        Ok(())
    }

    pub fn get_composite(&self, kind: &str, name: &str) -> CrossplaneResult<CompositeResource> {
        let key = Self::composite_key(kind, name);
        self.composites
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::CompositeNotFound(key))
    }

    pub fn list_composites(&self) -> Vec<CompositeResource> {
        self.composites.iter().map(|r| r.value().clone()).collect()
    }
}

impl Default for ClaimStore {
    fn default() -> Self {
        Self::new()
    }
}

//! Federation relationship store.

use crate::error::{SpireError, SpireResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub struct FederationStore {
    relationships: DashMap<String, FederationRelationship>,
}

impl FederationStore {
    pub fn new() -> Self {
        Self { relationships: DashMap::new() }
    }

    pub fn create(&self, req: CreateFederationRequest) -> SpireResult<FederationRelationship> {
        if self.relationships.contains_key(&req.trust_domain) {
            return Err(SpireError::AlreadyExists(req.trust_domain));
        }
        let rel = FederationRelationship {
            id: Uuid::new_v4(),
            trust_domain: req.trust_domain.clone(),
            bundle_endpoint_url: req.bundle_endpoint_url,
            bundle_endpoint_profile: req.bundle_endpoint_profile.unwrap_or(BundleEndpointProfile::HttpsWeb),
            status: FederationStatus::Active,
            last_bundle_refresh: None,
            created_at: Utc::now(),
        };
        self.relationships.insert(req.trust_domain, rel.clone());
        Ok(rel)
    }

    pub fn get(&self, trust_domain: &str) -> SpireResult<FederationRelationship> {
        self.relationships.get(trust_domain).map(|r| r.clone()).ok_or_else(|| SpireError::FederationNotFound(trust_domain.to_owned()))
    }

    pub fn list(&self) -> Vec<FederationRelationship> {
        self.relationships.iter().map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, trust_domain: &str) -> SpireResult<()> {
        self.relationships.remove(trust_domain).ok_or_else(|| SpireError::FederationNotFound(trust_domain.to_owned()))?;
        Ok(())
    }
}

impl Default for FederationStore {
    fn default() -> Self { Self::new() }
}

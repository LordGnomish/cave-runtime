//! Trust domain store.

use crate::error::{SpireError, SpireResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use tracing::info;
use uuid::Uuid;

pub struct TrustDomainStore {
    domains: DashMap<String, TrustDomain>,
}

impl TrustDomainStore {
    pub fn new() -> Self {
        Self { domains: DashMap::new() }
    }

    pub fn create(&self, req: CreateTrustDomainRequest) -> SpireResult<TrustDomain> {
        if self.domains.contains_key(&req.name) {
            return Err(SpireError::AlreadyExists(req.name));
        }
        let domain = TrustDomain {
            id: Uuid::new_v4(),
            spiffe_id: format!("spiffe://{}", req.name),
            bundle: Some(Bundle {
                trust_domain: req.name.clone(),
                jwt_authorities: vec![],
                x509_authorities: vec![],
                sequence_number: 1,
                refresh_hint_secs: 300,
            }),
            name: req.name.clone(),
            status: TrustDomainStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.domains.insert(req.name.clone(), domain.clone());
        info!(trust_domain = %req.name, "trust domain created");
        Ok(domain)
    }

    pub fn get(&self, name: &str) -> SpireResult<TrustDomain> {
        self.domains.get(name).map(|r| r.clone()).ok_or_else(|| SpireError::TrustDomainNotFound(name.to_owned()))
    }

    pub fn list(&self) -> Vec<TrustDomain> {
        self.domains.iter().map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, name: &str) -> SpireResult<()> {
        self.domains.remove(name).ok_or_else(|| SpireError::TrustDomainNotFound(name.to_owned()))?;
        Ok(())
    }
}

impl Default for TrustDomainStore {
    fn default() -> Self { Self::new() }
}

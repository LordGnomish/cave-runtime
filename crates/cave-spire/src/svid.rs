//! Registration entries and SVID issuance.

use crate::error::{SpireError, SpireResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub struct RegistrationStore {
    entries: DashMap<String, RegistrationEntry>,
}

impl RegistrationStore {
    pub fn new() -> Self {
        Self { entries: DashMap::new() }
    }

    pub fn create(&self, req: CreateRegistrationEntryRequest) -> SpireResult<RegistrationEntry> {
        let entry_id = Uuid::new_v4().to_string();
        let entry = RegistrationEntry {
            id: Uuid::new_v4(),
            entry_id: entry_id.clone(),
            spiffe_id: req.spiffe_id,
            parent_id: req.parent_id,
            selectors: req.selectors,
            dns_names: req.dns_names.unwrap_or_default(),
            federates_with: req.federates_with.unwrap_or_default(),
            admin: req.admin.unwrap_or(false),
            downstream: false,
            ttl_secs: req.ttl_secs.unwrap_or(3600),
            store_svid: false,
            revision_number: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.entries.insert(entry_id, entry.clone());
        Ok(entry)
    }

    pub fn get(&self, entry_id: &str) -> SpireResult<RegistrationEntry> {
        self.entries.get(entry_id).map(|r| r.clone()).ok_or_else(|| SpireError::EntryNotFound(entry_id.to_owned()))
    }

    pub fn list(&self, trust_domain: Option<&str>) -> Vec<RegistrationEntry> {
        self.entries.iter()
            .filter(|r| trust_domain.map_or(true, |td| r.value().spiffe_id.trust_domain == td))
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete(&self, entry_id: &str) -> SpireResult<()> {
        self.entries.remove(entry_id).ok_or_else(|| SpireError::EntryNotFound(entry_id.to_owned()))?;
        Ok(())
    }
}

impl Default for RegistrationStore {
    fn default() -> Self { Self::new() }
}

pub struct SvidStore {
    x509: DashMap<String, X509Svid>,
    jwt: DashMap<String, JwtSvid>,
}

impl SvidStore {
    pub fn new() -> Self {
        Self { x509: DashMap::new(), jwt: DashMap::new() }
    }

    pub fn mint_x509(&self, req: MintX509SvidRequest) -> SpireResult<X509Svid> {
        let ttl = req.ttl_secs.unwrap_or(3600);
        let svid = X509Svid {
            id: Uuid::new_v4(),
            spiffe_id: req.spiffe_id.clone(),
            cert_chain_pem: format!("-----BEGIN CERTIFICATE-----\n# simulated for {}\n-----END CERTIFICATE-----", req.spiffe_id),
            private_key_pem: "-----BEGIN EC PRIVATE KEY-----\n# simulated\n-----END EC PRIVATE KEY-----".into(),
            bundle: "-----BEGIN CERTIFICATE-----\n# simulated bundle\n-----END CERTIFICATE-----".into(),
            hint: None,
            expires_at: Utc::now() + chrono::Duration::try_seconds(ttl as i64).unwrap_or_default(),
            issued_at: Utc::now(),
        };
        self.x509.insert(svid.id.to_string(), svid.clone());
        Ok(svid)
    }

    pub fn mint_jwt(&self, req: MintJwtSvidRequest) -> SpireResult<JwtSvid> {
        let ttl = req.ttl_secs.unwrap_or(3600);
        let svid = JwtSvid {
            id: Uuid::new_v4(),
            spiffe_id: req.spiffe_id.clone(),
            token: format!("eyJhbGciOiJFUzI1NiJ9.{}.simulated_sig", req.spiffe_id.replace('/', "_")),
            hint: req.hint,
            audience: req.audience,
            expires_at: Utc::now() + chrono::Duration::try_seconds(ttl as i64).unwrap_or_default(),
            issued_at: Utc::now(),
        };
        self.jwt.insert(svid.id.to_string(), svid.clone());
        Ok(svid)
    }

    pub fn get_x509(&self, id: &str) -> SpireResult<X509Svid> {
        self.x509.get(id).map(|r| r.clone()).ok_or_else(|| SpireError::SvidNotFound(id.to_owned()))
    }

    pub fn get_jwt(&self, id: &str) -> SpireResult<JwtSvid> {
        self.jwt.get(id).map(|r| r.clone()).ok_or_else(|| SpireError::SvidNotFound(id.to_owned()))
    }

    pub fn list_x509_for_spiffe_id(&self, spiffe_id: &str) -> Vec<X509Svid> {
        self.x509.iter().filter(|r| r.value().spiffe_id == spiffe_id).map(|r| r.value().clone()).collect()
    }
}

impl Default for SvidStore {
    fn default() -> Self { Self::new() }
}

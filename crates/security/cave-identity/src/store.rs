// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Store interface boundary
// line-ported from pkg/server/datastore/datastore.go.
//
//! Persistence boundary — registration entries, bundles, attested nodes,
//! federation relationships.
//!
//! The default backend ([`MemStore`]) keeps everything in-memory and is the
//! one wired up by the routes layer. The SQLite-backed
//! [`SqliteStoreFacade`] is a Charter-scope_cut placeholder that delegates
//! the persistence layer to `cave_db::CavePool` once the orchestrator wires
//! it; today it forwards every call to a `MemStore`.

use crate::error::{IdentityError, Result};
use crate::models::{
    AttestedNode, Bundle, FederationRelationship, RegistrationEntry, SpiffeId, TrustDomain,
};
use crate::registration::InMemoryEntryStore;
use dashmap::DashMap;
use std::sync::Arc;

/// In-memory store — combines every persistence surface.
pub struct MemStore {
    pub entries: InMemoryEntryStore,
    pub bundles: Arc<DashMap<String, Bundle>>,
    pub agents: Arc<DashMap<String, AttestedNode>>,
    pub federations: Arc<DashMap<String, FederationRelationship>>,
}

impl Default for MemStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemStore {
    pub fn new() -> Self {
        Self {
            entries: InMemoryEntryStore::new(),
            bundles: Arc::new(DashMap::new()),
            agents: Arc::new(DashMap::new()),
            federations: Arc::new(DashMap::new()),
        }
    }

    /// Insert/replace a bundle keyed by its trust domain.
    pub fn put_bundle(&self, b: Bundle) -> Result<()> {
        self.bundles
            .insert(b.trust_domain.as_str().to_string(), b);
        Ok(())
    }

    pub fn get_bundle(&self, td: &TrustDomain) -> Result<Bundle> {
        self.bundles
            .get(td.as_str())
            .map(|b| b.clone())
            .ok_or_else(|| IdentityError::BundleNotFound(td.as_str().to_string()))
    }

    pub fn delete_bundle(&self, td: &TrustDomain) -> Result<Bundle> {
        self.bundles
            .remove(td.as_str())
            .map(|(_, b)| b)
            .ok_or_else(|| IdentityError::BundleNotFound(td.as_str().to_string()))
    }

    pub fn list_bundles(&self) -> Vec<Bundle> {
        self.bundles.iter().map(|b| b.value().clone()).collect()
    }

    /// Insert/replace an attested-node row.
    pub fn put_agent(&self, a: AttestedNode) -> Result<()> {
        self.agents.insert(a.spiffe_id.as_str().to_string(), a);
        Ok(())
    }

    pub fn get_agent(&self, id: &SpiffeId) -> Result<AttestedNode> {
        self.agents
            .get(id.as_str())
            .map(|a| a.clone())
            .ok_or_else(|| IdentityError::Internal(format!("agent not found: {}", id)))
    }

    pub fn list_agents(&self) -> Vec<AttestedNode> {
        self.agents.iter().map(|a| a.value().clone()).collect()
    }

    /// Mark an agent as banned — issued SVID requests will reject.
    pub fn ban_agent(&self, id: &SpiffeId) -> Result<()> {
        let mut entry = self
            .agents
            .get_mut(id.as_str())
            .ok_or_else(|| IdentityError::Internal(format!("agent not found: {}", id)))?;
        entry.banned = true;
        Ok(())
    }

    pub fn put_federation(&self, r: FederationRelationship) -> Result<()> {
        self.federations
            .insert(r.trust_domain.as_str().to_string(), r);
        Ok(())
    }

    pub fn get_federation(&self, td: &TrustDomain) -> Result<FederationRelationship> {
        self.federations
            .get(td.as_str())
            .map(|r| r.clone())
            .ok_or_else(|| IdentityError::FederationInvalid(format!("no fed: {}", td.as_str())))
    }

    pub fn list_federations(&self) -> Vec<FederationRelationship> {
        self.federations
            .iter()
            .map(|f| f.value().clone())
            .collect()
    }

    pub fn delete_federation(&self, td: &TrustDomain) -> Result<FederationRelationship> {
        self.federations
            .remove(td.as_str())
            .map(|(_, r)| r)
            .ok_or_else(|| IdentityError::FederationInvalid(td.as_str().to_string()))
    }
}

/// SQLite-backed facade — Charter-scope_cut; today delegates to
/// [`MemStore`]. When [`cave_db::CavePool`] is wired the persistence calls
/// route through it without changing this trait.
pub struct SqliteStoreFacade {
    inner: MemStore,
    /// Logical DB URL (for debug); unused until cave_db wiring lands.
    pub db_url: String,
}

impl SqliteStoreFacade {
    pub fn new(db_url: impl Into<String>) -> Self {
        Self {
            inner: MemStore::new(),
            db_url: db_url.into(),
        }
    }
    pub fn create_entry(&self, e: RegistrationEntry) -> Result<RegistrationEntry> {
        self.inner.entries.create(e)
    }
    pub fn list_entries(&self) -> Vec<RegistrationEntry> {
        self.inner.entries.list()
    }
    pub fn store(&self) -> &MemStore {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn bundle(td: &str) -> Bundle {
        Bundle {
            trust_domain: TrustDomain::new(td),
            x509_authorities: vec![],
            jwt_authorities: vec![],
            refresh_hint_seconds: 60,
            sequence_number: 1,
        }
    }

    fn agent(id: &str) -> AttestedNode {
        AttestedNode {
            spiffe_id: SpiffeId::new(id),
            attestation_type: "k8s_psat".into(),
            serial_number: "1".into(),
            cert_not_after: Utc::now(),
            new_serial_number: None,
            new_cert_not_after: None,
            banned: false,
            selectors: vec![],
        }
    }

    #[test]
    fn bundle_crud() {
        let s = MemStore::new();
        let b = bundle("example.org");
        s.put_bundle(b.clone()).unwrap();
        assert_eq!(
            s.get_bundle(&b.trust_domain).unwrap().sequence_number,
            1
        );
        assert_eq!(s.list_bundles().len(), 1);
        s.delete_bundle(&b.trust_domain).unwrap();
        assert!(s.get_bundle(&b.trust_domain).is_err());
    }

    #[test]
    fn agent_ban() {
        let s = MemStore::new();
        let a = agent("spiffe://example.org/spire/agent/k8s_psat/n");
        s.put_agent(a.clone()).unwrap();
        assert!(!s.get_agent(&a.spiffe_id).unwrap().banned);
        s.ban_agent(&a.spiffe_id).unwrap();
        assert!(s.get_agent(&a.spiffe_id).unwrap().banned);
    }

    #[test]
    fn missing_bundle_errors() {
        let s = MemStore::new();
        assert!(matches!(
            s.get_bundle(&TrustDomain::new("missing.org")),
            Err(IdentityError::BundleNotFound(_))
        ));
    }

    #[test]
    fn sqlite_facade_holds_db_url() {
        let f = SqliteStoreFacade::new("sqlite:///tmp/spire.db");
        assert_eq!(f.db_url, "sqlite:///tmp/spire.db");
        assert!(f.list_entries().is_empty());
    }
}

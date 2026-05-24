// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Federation flow + endpoint
// handshake shape line-ported from pkg/server/endpoints/bundle/server.go +
// pkg/server/bundle/client/client.go.
//
//! SPIFFE trust-domain federation — peer bundle exchange.

use crate::bundle::{self, BundleDoc};
use crate::error::{IdentityError, Result};
use crate::models::{Bundle, BundleEndpointProfile, FederationRelationship, TrustDomain};
use crate::spiffe_id::parse_spiffe_id;
use crate::store::MemStore;
use std::sync::Arc;

/// Federation manager — encapsulates the relationship table + an injected
/// fetcher used to refresh remote bundles.
pub struct FederationManager {
    store: Arc<MemStore>,
    /// Pluggable bundle fetcher. Tests inject a deterministic table.
    fetcher: Arc<dyn BundleFetcher>,
}

#[async_trait::async_trait]
pub trait BundleFetcher: Send + Sync {
    async fn fetch(&self, td: &TrustDomain, endpoint_url: &str) -> Result<BundleDoc>;
}

/// In-memory bundle fetcher — every `(trust_domain, endpoint_url)` pair
/// returns the pre-seeded document.
pub struct StubBundleFetcher {
    pub by_url: dashmap::DashMap<String, BundleDoc>,
}

impl Default for StubBundleFetcher {
    fn default() -> Self {
        Self {
            by_url: dashmap::DashMap::new(),
        }
    }
}

#[async_trait::async_trait]
impl BundleFetcher for StubBundleFetcher {
    async fn fetch(&self, _td: &TrustDomain, endpoint_url: &str) -> Result<BundleDoc> {
        self.by_url
            .get(endpoint_url)
            .map(|v| v.clone())
            .ok_or_else(|| IdentityError::FederationUnreachable(endpoint_url.to_string()))
    }
}

impl FederationManager {
    pub fn new(store: Arc<MemStore>, fetcher: Arc<dyn BundleFetcher>) -> Self {
        Self { store, fetcher }
    }

    /// Create a federation relationship. The relationship is stored regardless
    /// of whether the initial bundle fetch succeeds; clients should `refresh`
    /// before issuing federated SVIDs.
    pub fn create(&self, rel: FederationRelationship) -> Result<FederationRelationship> {
        validate(&rel)?;
        self.store.put_federation(rel.clone())?;
        Ok(rel)
    }

    /// Lookup a relationship by trust domain.
    pub fn get(&self, td: &TrustDomain) -> Result<FederationRelationship> {
        self.store.get_federation(td)
    }

    pub fn delete(&self, td: &TrustDomain) -> Result<FederationRelationship> {
        self.store.delete_federation(td)
    }

    pub fn list(&self) -> Vec<FederationRelationship> {
        self.store.list_federations()
    }

    /// Refresh a relationship's bundle by fetching the peer endpoint.
    pub async fn refresh(&self, td: &TrustDomain) -> Result<Bundle> {
        let mut rel = self.store.get_federation(td)?;
        let doc = self.fetcher.fetch(td, &rel.bundle_endpoint_url).await?;
        let b = bundle::unmarshal(td, &doc)?;
        self.store.put_bundle(b.clone())?;
        rel.trust_domain_bundle = Some(b.clone());
        self.store.put_federation(rel)?;
        Ok(b)
    }

    /// Verify a federated bundle against the relationship's expected profile.
    pub fn verify_bundle(
        &self,
        rel: &FederationRelationship,
        bundle: &Bundle,
    ) -> Result<()> {
        if bundle.trust_domain != rel.trust_domain {
            return Err(IdentityError::FederationInvalid(format!(
                "trust domain mismatch: want={} got={}",
                rel.trust_domain.as_str(),
                bundle.trust_domain.as_str()
            )));
        }
        if let BundleEndpointProfile::HttpsSpiffe { endpoint_spiffe_id } = &rel.bundle_endpoint_profile {
            parse_spiffe_id(endpoint_spiffe_id.as_str())?;
        }
        Ok(())
    }
}

fn validate(rel: &FederationRelationship) -> Result<()> {
    if rel.bundle_endpoint_url.is_empty() {
        return Err(IdentityError::FederationInvalid(
            "endpoint url empty".into(),
        ));
    }
    if !rel.bundle_endpoint_url.starts_with("https://") {
        return Err(IdentityError::FederationInvalid(
            "endpoint url must be https://".into(),
        ));
    }
    if rel.trust_domain.as_str().is_empty() {
        return Err(IdentityError::FederationInvalid("trust domain empty".into()));
    }
    if let BundleEndpointProfile::HttpsSpiffe { endpoint_spiffe_id } = &rel.bundle_endpoint_profile {
        parse_spiffe_id(endpoint_spiffe_id.as_str())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SpiffeId;

    fn rel(td: &str) -> FederationRelationship {
        FederationRelationship {
            trust_domain: TrustDomain::new(td),
            bundle_endpoint_url: format!("https://{}/bundle", td),
            bundle_endpoint_profile: BundleEndpointProfile::HttpsWeb,
            trust_domain_bundle: None,
        }
    }

    #[tokio::test]
    async fn create_and_get() {
        let store = Arc::new(MemStore::new());
        let f = Arc::new(StubBundleFetcher::default());
        let m = FederationManager::new(store, f);
        m.create(rel("peer.org")).unwrap();
        assert_eq!(m.get(&TrustDomain::new("peer.org")).unwrap().bundle_endpoint_url,
                   "https://peer.org/bundle");
    }

    #[tokio::test]
    async fn refresh_pulls_via_fetcher() {
        let store = Arc::new(MemStore::new());
        let stub = Arc::new(StubBundleFetcher::default());
        let doc = BundleDoc {
            keys: vec![],
            spiffe_refresh_hint: 60,
            spiffe_sequence: 5,
        };
        stub.by_url.insert("https://peer.org/bundle".into(), doc);
        let m = FederationManager::new(store, stub);
        m.create(rel("peer.org")).unwrap();
        let b = m.refresh(&TrustDomain::new("peer.org")).await.unwrap();
        assert_eq!(b.sequence_number, 5);
        assert_eq!(b.trust_domain.as_str(), "peer.org");
    }

    #[tokio::test]
    async fn refresh_missing_endpoint_errors() {
        let store = Arc::new(MemStore::new());
        let stub = Arc::new(StubBundleFetcher::default());
        let m = FederationManager::new(store, stub);
        m.create(rel("peer.org")).unwrap();
        assert!(m
            .refresh(&TrustDomain::new("peer.org"))
            .await
            .is_err());
    }

    #[test]
    fn create_rejects_non_https() {
        let store = Arc::new(MemStore::new());
        let m = FederationManager::new(store, Arc::new(StubBundleFetcher::default()));
        let mut r = rel("peer.org");
        r.bundle_endpoint_url = "http://peer.org".into();
        assert!(m.create(r).is_err());
    }

    #[test]
    fn create_rejects_bad_spiffe_endpoint() {
        let store = Arc::new(MemStore::new());
        let m = FederationManager::new(store, Arc::new(StubBundleFetcher::default()));
        let mut r = rel("peer.org");
        r.bundle_endpoint_profile = BundleEndpointProfile::HttpsSpiffe {
            endpoint_spiffe_id: SpiffeId::new("not-valid"),
        };
        assert!(m.create(r).is_err());
    }

    #[test]
    fn verify_rejects_td_mismatch() {
        let store = Arc::new(MemStore::new());
        let m = FederationManager::new(store, Arc::new(StubBundleFetcher::default()));
        let r = rel("peer.org");
        let bundle = Bundle {
            trust_domain: TrustDomain::new("other.org"),
            x509_authorities: vec![],
            jwt_authorities: vec![],
            refresh_hint_seconds: 0,
            sequence_number: 0,
        };
        assert!(m.verify_bundle(&r, &bundle).is_err());
    }

    #[tokio::test]
    async fn delete_removes() {
        let store = Arc::new(MemStore::new());
        let m = FederationManager::new(store, Arc::new(StubBundleFetcher::default()));
        m.create(rel("peer.org")).unwrap();
        m.delete(&TrustDomain::new("peer.org")).unwrap();
        assert!(m.list().is_empty());
    }
}

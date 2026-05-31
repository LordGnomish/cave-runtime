// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Workload API handler logic
// line-ported from pkg/agent/endpoints/workload/handler.go — the in-process
// request handling, NOT the gRPC/UDS transport (owned by cave-mesh).
//
//! SPIFFE Workload API — in-process handler.
//!
//! Ports the request-handling core of `pkg/agent/endpoints/workload`: given a
//! caller's attested selector set, find every registration entry the workload
//! is *entitled* to (`MATCH_SUBSET` — the entry's selectors must all be
//! present in the caller's set), then mint the corresponding X.509-SVIDs /
//! JWT-SVIDs and assemble the trust-bundle map (own trust domain + each
//! `federates_with` peer).
//!
//! The gRPC service + the `SPIFFE_ENDPOINT_SOCKET` Unix-domain-socket
//! transport are a Charter-scope_cut owned by cave-mesh's data plane; this
//! module is what that transport calls into.

use crate::error::{IdentityError, Result};
use crate::jwt_svid;
use crate::models::{Bundle, JwtSvid, RegistrationEntry, Selector, SpiffeId, X509Svid};
use crate::registration::{selectors_match, InMemoryEntryStore};
use crate::server_ca::ServerCa;
use crate::x509_svid;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Response for `FetchX509SVID` — the authorized leaf SVIDs plus the trust
/// bundles (own + federated) keyed by trust-domain name.
#[derive(Debug, Clone)]
pub struct X509SvidResponse {
    pub svids: Vec<X509Svid>,
    pub federated_bundles: BTreeMap<String, Bundle>,
}

/// Response for `FetchJWTSVID`.
#[derive(Debug, Clone)]
pub struct JwtSvidResponse {
    pub svids: Vec<JwtSvid>,
}

/// In-process SPIFFE Workload API handler.
pub struct WorkloadApiHandler {
    ca: Arc<ServerCa>,
    store: Arc<InMemoryEntryStore>,
}

impl WorkloadApiHandler {
    pub fn new(ca: Arc<ServerCa>, store: Arc<InMemoryEntryStore>) -> Self {
        Self { ca, store }
    }

    /// Entries the caller is entitled to: an entry applies when every one of
    /// its selectors is present in the caller's attested selector set
    /// (`MATCH_SUBSET`, mirroring `pkg/agent/manager.Cache` authorization).
    fn authorized_entries(&self, caller_selectors: &[Selector]) -> Vec<RegistrationEntry> {
        self.store
            .list()
            .into_iter()
            .filter(|e| !e.selectors.is_empty() && selectors_match(&e.selectors, caller_selectors))
            .collect()
    }

    /// Assemble the trust-bundle map: the agent's own trust-domain bundle plus
    /// an (empty-authority) placeholder for every federated trust domain the
    /// authorized entries reference.
    fn bundle_map(&self, entries: &[RegistrationEntry]) -> BTreeMap<String, Bundle> {
        let own = self.ca.trust_bundle();
        let mut map = BTreeMap::new();
        map.insert(own.trust_domain.as_str().to_string(), own);
        for e in entries {
            for td in &e.federates_with {
                map.entry(td.as_str().to_string()).or_insert_with(|| Bundle {
                    trust_domain: td.clone(),
                    x509_authorities: Vec::new(),
                    jwt_authorities: Vec::new(),
                    refresh_hint_seconds: 0,
                    sequence_number: 0,
                });
            }
        }
        map
    }

    /// `FetchX509SVID` — mint a leaf SVID per authorized entry + bundle map.
    pub fn fetch_x509_svid(&self, caller_selectors: &[Selector]) -> Result<X509SvidResponse> {
        let entries = self.authorized_entries(caller_selectors);
        let mut svids = Vec::with_capacity(entries.len());
        for e in &entries {
            svids.push(x509_svid::issue(&self.ca, e)?);
        }
        let federated_bundles = self.bundle_map(&entries);
        Ok(X509SvidResponse {
            svids,
            federated_bundles,
        })
    }

    /// `FetchX509Bundles` — just the trust-bundle map for the caller.
    pub fn fetch_x509_bundles(&self, caller_selectors: &[Selector]) -> BTreeMap<String, Bundle> {
        let entries = self.authorized_entries(caller_selectors);
        self.bundle_map(&entries)
    }

    /// `FetchJWTSVID` over every authorized entry for the given audiences.
    pub fn fetch_jwt_svid(
        &self,
        caller_selectors: &[Selector],
        audience: &[String],
    ) -> Result<JwtSvidResponse> {
        self.fetch_jwt_svid_for(caller_selectors, audience, None)
    }

    /// `FetchJWTSVID` restricted to a single SPIFFE ID when `spiffe_id` is set
    /// — the workload requests a token for one of its identities. An
    /// unauthorized SPIFFE ID simply yields no SVIDs.
    pub fn fetch_jwt_svid_for(
        &self,
        caller_selectors: &[Selector],
        audience: &[String],
        spiffe_id: Option<&SpiffeId>,
    ) -> Result<JwtSvidResponse> {
        if audience.is_empty() {
            return Err(IdentityError::JwtInvalid("audience required".into()));
        }
        let mut svids = Vec::new();
        for e in self.authorized_entries(caller_selectors) {
            if let Some(want) = spiffe_id {
                if &e.spiffe_id != want {
                    continue;
                }
            }
            svids.push(jwt_svid::issue(&self.ca, &e, audience.to_vec())?);
        }
        Ok(JwtSvidResponse { svids })
    }

    /// `ValidateJWTSVID` — verify a token against the agent's own bundle.
    pub fn validate_jwt_svid(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<crate::models::JwtSvidClaims> {
        jwt_svid::verify(token, audience, &self.ca.trust_bundle())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Selector, SpiffeId, TrustDomain};
    use crate::registration::InMemoryEntryStore;
    use crate::server_ca::{RotationParams, ServerCa};
    use crate::models::RegistrationEntry;
    use chrono::Utc;
    use std::sync::Arc;

    fn setup() -> (Arc<ServerCa>, Arc<InMemoryEntryStore>) {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        let store = InMemoryEntryStore::new();
        // entry bound to k8s ns:default + sa:web
        store
            .create(RegistrationEntry {
                spiffe_id: SpiffeId::new("spiffe://example.org/web"),
                parent_id: SpiffeId::new("spiffe://example.org/spire/agent/k8s_psat/n1"),
                selectors: vec![
                    Selector::new("k8s", "ns:default"),
                    Selector::new("k8s", "sa:web"),
                ],
                federates_with: vec![TrustDomain::new("peer.org")],
                ..Default::default()
            })
            .unwrap();
        // a second entry the workload should NOT get (different selector)
        store
            .create(RegistrationEntry {
                spiffe_id: SpiffeId::new("spiffe://example.org/db"),
                parent_id: SpiffeId::new("spiffe://example.org/spire/agent/k8s_psat/n1"),
                selectors: vec![Selector::new("k8s", "sa:db")],
                ..Default::default()
            })
            .unwrap();
        (Arc::new(ca), Arc::new(store))
    }

    fn caller() -> Vec<Selector> {
        vec![
            Selector::new("k8s", "ns:default"),
            Selector::new("k8s", "sa:web"),
            Selector::new("unix", "uid:1000"),
        ]
    }

    #[test]
    fn fetch_x509_returns_only_authorized_svids() {
        let (ca, store) = setup();
        let h = WorkloadApiHandler::new(ca, store);
        let resp = h.fetch_x509_svid(&caller()).unwrap();
        // only the web entry matches the caller's selector set
        assert_eq!(resp.svids.len(), 1);
        assert_eq!(resp.svids[0].spiffe_id.as_str(), "spiffe://example.org/web");
        // federated trust domains surface in the response bundle map
        assert!(resp.federated_bundles.contains_key("peer.org"));
        // own trust domain bundle present
        assert!(resp.federated_bundles.contains_key("example.org"));
    }

    #[test]
    fn fetch_x509_empty_when_no_match() {
        let (ca, store) = setup();
        let h = WorkloadApiHandler::new(ca, store);
        let resp = h.fetch_x509_svid(&[Selector::new("k8s", "ns:other")]).unwrap();
        assert!(resp.svids.is_empty());
    }

    #[test]
    fn fetch_jwt_returns_token_for_audience() {
        let (ca, store) = setup();
        let h = WorkloadApiHandler::new(ca, store);
        let resp = h
            .fetch_jwt_svid(&caller(), &["api.example".to_string()])
            .unwrap();
        assert_eq!(resp.svids.len(), 1);
        assert_eq!(resp.svids[0].spiffe_id.as_str(), "spiffe://example.org/web");
        assert!(resp.svids[0].audience.contains(&"api.example".to_string()));
    }

    #[test]
    fn fetch_jwt_requires_audience() {
        let (ca, store) = setup();
        let h = WorkloadApiHandler::new(ca, store);
        assert!(h.fetch_jwt_svid(&caller(), &[]).is_err());
    }

    #[test]
    fn fetch_jwt_for_specific_spiffe_id() {
        let (ca, store) = setup();
        let h = WorkloadApiHandler::new(ca, store);
        // ask only for a specific SPIFFE ID among the authorized set
        let resp = h
            .fetch_jwt_svid_for(
                &caller(),
                &["api.example".to_string()],
                Some(&SpiffeId::new("spiffe://example.org/web")),
            )
            .unwrap();
        assert_eq!(resp.svids.len(), 1);
        // asking for an unauthorized SPIFFE ID yields nothing
        let resp2 = h
            .fetch_jwt_svid_for(
                &caller(),
                &["api.example".to_string()],
                Some(&SpiffeId::new("spiffe://example.org/db")),
            )
            .unwrap();
        assert!(resp2.svids.is_empty());
    }

    #[test]
    fn validate_jwt_roundtrip() {
        let (ca, store) = setup();
        let h = WorkloadApiHandler::new(ca, store);
        let issued = h
            .fetch_jwt_svid(&caller(), &["api.example".to_string()])
            .unwrap();
        let token = &issued.svids[0].token;
        let claims = h.validate_jwt_svid(token, "api.example").unwrap();
        assert_eq!(claims.sub, "spiffe://example.org/web");
        // wrong audience rejected
        assert!(h.validate_jwt_svid(token, "wrong.aud").is_err());
    }

    #[test]
    fn fetch_x509_bundles_has_own_and_federated() {
        let (ca, store) = setup();
        let h = WorkloadApiHandler::new(ca, store);
        let bundles = h.fetch_x509_bundles(&caller());
        assert!(bundles.contains_key("example.org"));
        // peer.org only appears once the workload is entitled to a federated entry
        assert!(bundles.contains_key("peer.org"));
    }
}

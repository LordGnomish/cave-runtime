// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Workload API handler logic
// line-ported from pkg/agent/endpoints/workload/handler.go — the in-process
// request handling, NOT the gRPC/UDS transport (owned by cave-mesh).

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

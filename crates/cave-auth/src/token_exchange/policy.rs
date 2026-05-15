// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/TokenExchangeGrantType.java#permissions + RFC 8693 §4
//
//! Exchange policy — which clients may exchange to which audiences.
//!
//! Keycloak's TokenExchangeGrantType enforces this via realm-management
//! permissions; we keep the same model: a `(client_id, target_aud)` allow-list.

use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny,
}

pub struct ExchangePolicy {
    /// `(client_id, aud)` pairs that are explicitly allowed.
    grants: Mutex<HashSet<(String, String)>>,
    /// `client_id`s that may exchange to ANY audience (super-admin role).
    wildcard_clients: Mutex<HashSet<String>>,
}

impl Default for ExchangePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ExchangePolicy {
    pub fn new() -> Self {
        Self {
            grants: Mutex::new(HashSet::new()),
            wildcard_clients: Mutex::new(HashSet::new()),
        }
    }

    pub fn allow(&self, client_id: &str, audience: &str) {
        self.grants
            .lock()
            .unwrap()
            .insert((client_id.to_string(), audience.to_string()));
    }

    pub fn allow_any_audience(&self, client_id: &str) {
        self.wildcard_clients
            .lock()
            .unwrap()
            .insert(client_id.to_string());
    }

    pub fn revoke(&self, client_id: &str, audience: &str) {
        self.grants
            .lock()
            .unwrap()
            .remove(&(client_id.to_string(), audience.to_string()));
    }

    pub fn decide(&self, client_id: &str, audience: &str) -> PolicyDecision {
        if self
            .wildcard_clients
            .lock()
            .unwrap()
            .contains(client_id)
        {
            return PolicyDecision::Allow;
        }
        if self
            .grants
            .lock()
            .unwrap()
            .contains(&(client_id.to_string(), audience.to_string()))
        {
            return PolicyDecision::Allow;
        }
        PolicyDecision::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_pair_denied() {
        let p = ExchangePolicy::new();
        assert_eq!(p.decide("c1", "aud-x"), PolicyDecision::Deny);
    }

    #[test]
    fn explicit_grant_allows() {
        let p = ExchangePolicy::new();
        p.allow("c1", "aud-x");
        assert_eq!(p.decide("c1", "aud-x"), PolicyDecision::Allow);
    }

    #[test]
    fn grant_is_pair_specific() {
        let p = ExchangePolicy::new();
        p.allow("c1", "aud-x");
        assert_eq!(p.decide("c1", "aud-y"), PolicyDecision::Deny);
        assert_eq!(p.decide("c2", "aud-x"), PolicyDecision::Deny);
    }

    #[test]
    fn wildcard_client_allows_any_audience() {
        let p = ExchangePolicy::new();
        p.allow_any_audience("super");
        assert_eq!(p.decide("super", "anything"), PolicyDecision::Allow);
        assert_eq!(p.decide("super", "other"), PolicyDecision::Allow);
    }

    #[test]
    fn revoke_removes_grant() {
        let p = ExchangePolicy::new();
        p.allow("c1", "aud-x");
        p.revoke("c1", "aud-x");
        assert_eq!(p.decide("c1", "aud-x"), PolicyDecision::Deny);
    }
}

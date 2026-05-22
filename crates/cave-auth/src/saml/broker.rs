// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SAML flow state machines — SP-initiated and IdP-initiated.
//!
//! Mirrors `org.keycloak.broker.saml.SAMLEndpoint` (SP role) +
//! `org.keycloak.protocol.saml.SamlService` (IdP role) from
//! upstream Keycloak. cave-auth wears either hat depending on
//! deployment:
//!
//! * **SP role** — cave federates *out* to a customer's IdP.
//!   `start_sp_initiated_login` mints an `AuthnRequest`,
//!   stashes the in-flight state, and returns the
//!   redirect URL. When the IdP POSTs back, `process_response`
//!   verifies + extracts the [`SamlSubject`].
//! * **IdP role** — cave acts as the IdP for a downstream
//!   service. `accept_authn_request` parses the inbound
//!   request and stashes its `ID` so the response can carry it
//!   as `InResponseTo`. `mint_response` builds the
//!   Assertion + signs it.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::Utc;

use super::authn_request::AuthnRequest;
use super::binding::{BINDING_POST, BINDING_REDIRECT, redirect_encode};
use super::response::{Assertion, Response, StatusCode};
use super::{SamlError, SamlSubject};

/// One in-flight SP-initiated login. Stashed at request-mint
/// time and looked up at response-process time so the broker
/// can match `InResponseTo` and reject responses with no
/// matching request.
#[derive(Debug, Clone)]
struct InFlight {
    /// IdP entity ID we sent the request to.
    idp_entity_id: String,
    /// When the request was issued. Used to garbage-collect
    /// stale entries (a real IdP responds in seconds; cave
    /// keeps state for 5 minutes).
    created_at: chrono::DateTime<Utc>,
}

/// The thing the auth_middleware layer interacts with. Holds
/// SP-side broker state (in-flight requests). Send + Sync so the
/// same instance can serve concurrent HTTP handlers.
#[derive(Clone, Default)]
pub struct SamlBroker {
    in_flight: Arc<RwLock<HashMap<String, InFlight>>>,
    /// How long after `created_at` an in-flight entry is
    /// pruned. 5 minutes matches Keycloak's
    /// `EXPECTED_LOGIN_TIMEOUT` default.
    ttl_seconds: i64,
}

impl SamlBroker {
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(RwLock::new(HashMap::new())),
            ttl_seconds: 300,
        }
    }

    /// Returns the broker's in-flight TTL. Used by tests.
    pub fn ttl_seconds(&self) -> i64 {
        self.ttl_seconds
    }

    /// SP role: start an SP-initiated login. Mints an
    /// `AuthnRequest`, stashes its ID, and returns the
    /// `(redirect_url, request_id)` pair. `idp_sso_url` is the
    /// IdP's `SingleSignOnService` endpoint (the `Location`
    /// from its metadata).
    pub fn start_sp_initiated_login(
        &self,
        sp_entity_id: impl Into<String>,
        idp_sso_url: impl Into<String>,
        idp_entity_id: impl Into<String>,
        acs_url: impl Into<String>,
    ) -> Result<(String, String), SamlError> {
        let idp_sso_url = idp_sso_url.into();
        let idp_entity_id = idp_entity_id.into();
        let req = AuthnRequest::new(sp_entity_id, &idp_sso_url).with_acs_url(acs_url);
        let id = req.id.clone();

        self.in_flight.write().expect("poisoned").insert(
            id.clone(),
            InFlight {
                idp_entity_id,
                created_at: Utc::now(),
            },
        );

        let xml = req.to_xml()?;
        let encoded = redirect_encode(&xml)?;
        let url = format!(
            "{}{}SAMLRequest={}",
            idp_sso_url,
            if idp_sso_url.contains('?') { '&' } else { '?' },
            urlencode_minimal(&encoded)
        );
        Ok((url, id))
    }

    /// SP role: process an IdP's Response. Verifies status,
    /// matches `InResponseTo` against in-flight state, checks
    /// the Assertion's validity window, and extracts the
    /// [`SamlSubject`]. The signature-verification step is
    /// the *caller's* responsibility — the broker exposes the
    /// raw `Response` first so the auth_middleware can plug in
    /// its own key resolution (per-IdP cert from metadata).
    pub fn process_response(&self, response: Response) -> Result<SamlSubject, SamlError> {
        if response.status != StatusCode::Success {
            return Err(SamlError::Other(format!(
                "Response status is not Success: {}",
                response.status.as_urn()
            )));
        }

        let irt = response
            .in_response_to
            .as_deref()
            .ok_or_else(|| SamlError::MissingField("InResponseTo".into()))?;

        let entry = self
            .in_flight
            .write()
            .expect("poisoned")
            .remove(irt)
            .ok_or_else(|| {
                SamlError::WrongDestination(format!("no in-flight request for {irt}"))
            })?;

        if (Utc::now() - entry.created_at).num_seconds() > self.ttl_seconds {
            return Err(SamlError::Expired);
        }

        let assertion = response
            .assertion
            .as_ref()
            .ok_or_else(|| SamlError::MissingField("Assertion".into()))?;
        if !assertion.is_time_valid(Utc::now()) {
            return Err(SamlError::Expired);
        }

        // Issuer of the assertion must match what we sent to.
        if assertion.issuer != entry.idp_entity_id {
            return Err(SamlError::WrongDestination(format!(
                "Assertion issuer {} != expected {}",
                assertion.issuer, entry.idp_entity_id
            )));
        }

        response.into_subject()
    }

    /// IdP role: accept an inbound `AuthnRequest`. Validates the
    /// `Destination` field and returns the request — the caller
    /// then prompts the user, and on success calls
    /// [`Self::mint_response`] with the resulting subject.
    pub fn accept_authn_request(
        &self,
        request: AuthnRequest,
        my_sso_endpoint: &str,
    ) -> Result<AuthnRequest, SamlError> {
        if !request.destination.is_empty() && request.destination != my_sso_endpoint {
            return Err(SamlError::WrongDestination(format!(
                "AuthnRequest Destination {} != my SSO endpoint {}",
                request.destination, my_sso_endpoint
            )));
        }
        Ok(request)
    }

    /// IdP role: build a Response for the given subject. The
    /// caller signs it externally (see [`super::signature`])
    /// before sending it over the chosen binding.
    pub fn mint_response(
        &self,
        idp_entity_id: impl Into<String>,
        request: &AuthnRequest,
        subject: SamlSubject,
    ) -> Response {
        let mut a = Assertion::new(idp_entity_id, subject.name_id);
        a.subject_name_id_format = subject.name_id_format;
        a.session_index = subject.session_index;
        for aud in [request.issuer.as_str()] {
            a = a.with_audience(aud);
        }
        for (k, vs) in subject.attributes {
            for v in vs {
                a = a.with_attribute(k.clone(), v);
            }
        }

        let acs = request
            .acs_url
            .clone()
            .unwrap_or_else(|| request.issuer.clone());
        Response::success(a.issuer.clone(), acs, Some(request.id.clone()), a)
    }

    /// Garbage-collect expired in-flight entries. Cheap to call
    /// frequently — no global lock outside the brief write.
    pub fn gc(&self) {
        let cutoff = Utc::now() - chrono::Duration::seconds(self.ttl_seconds);
        let mut g = self.in_flight.write().expect("poisoned");
        g.retain(|_, v| v.created_at >= cutoff);
    }

    /// Live in-flight count — handy for metrics + tests.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.read().expect("poisoned").len()
    }

    /// Which transport bindings cave-auth supports as an IdP.
    /// Matches Keycloak: both Redirect and POST in-bound.
    pub fn supported_bindings(&self) -> [&'static str; 2] {
        [BINDING_REDIRECT, BINDING_POST]
    }
}

/// Minimal URL-encoder. SAML query values are limited to the
/// base64 alphabet + `=` + `/+` — only those three need
/// percent-encoding. Avoiding a `url` crate dep for one call
/// site.
fn urlencode_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'+' => out.push_str("%2B"),
            b'/' => out.push_str("%2F"),
            b'=' => out.push_str("%3D"),
            _ => out.push(b as char),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saml::NameIdFormat;
    use std::collections::BTreeMap;

    fn fake_subject() -> SamlSubject {
        let mut attrs = BTreeMap::new();
        attrs.insert("email".into(), vec!["alice@example.com".into()]);
        SamlSubject {
            name_id: "alice@example.com".into(),
            name_id_format: NameIdFormat::EmailAddress,
            issuer: "https://idp.example".into(),
            attributes: attrs,
            session_index: Some("idx-1".into()),
        }
    }

    #[test]
    fn start_sp_initiated_login_returns_redirect_url_and_tracks_state() {
        let b = SamlBroker::new();
        let (url, id) = b
            .start_sp_initiated_login(
                "https://sp.cave",
                "https://idp.example/sso",
                "https://idp.example",
                "https://sp.cave/acs",
            )
            .unwrap();
        assert!(url.starts_with("https://idp.example/sso?SAMLRequest="));
        assert!(id.starts_with('_'));
        assert_eq!(b.in_flight_count(), 1);
    }

    #[test]
    fn process_response_consumes_in_flight() {
        let b = SamlBroker::new();
        let (_url, req_id) = b
            .start_sp_initiated_login(
                "https://sp.cave",
                "https://idp.example/sso",
                "https://idp.example",
                "https://sp.cave/acs",
            )
            .unwrap();
        let a = Assertion::new("https://idp.example", "alice@example.com")
            .with_audience("https://sp.cave");
        let r = Response::success(
            "https://idp.example",
            "https://sp.cave/acs",
            Some(req_id),
            a,
        );
        let subj = b.process_response(r).unwrap();
        assert_eq!(subj.name_id, "alice@example.com");
        // In-flight entry consumed.
        assert_eq!(b.in_flight_count(), 0);
    }

    #[test]
    fn process_response_rejects_unknown_in_response_to() {
        let b = SamlBroker::new();
        let a = Assertion::new("https://idp.example", "alice@example.com");
        let r = Response::success(
            "https://idp.example",
            "https://sp.cave/acs",
            Some("_never-issued".to_string()),
            a,
        );
        assert!(matches!(
            b.process_response(r).unwrap_err(),
            SamlError::WrongDestination(_)
        ));
    }

    #[test]
    fn process_response_rejects_mismatched_issuer() {
        let b = SamlBroker::new();
        let (_url, req_id) = b
            .start_sp_initiated_login(
                "https://sp.cave",
                "https://idp.example/sso",
                "https://idp.expected",
                "https://sp.cave/acs",
            )
            .unwrap();
        // Assertion signed by a DIFFERENT issuer than the broker stored.
        let a = Assertion::new("https://idp.imposter", "alice");
        let r = Response::success(
            "https://idp.imposter",
            "https://sp.cave/acs",
            Some(req_id),
            a,
        );
        assert!(matches!(
            b.process_response(r).unwrap_err(),
            SamlError::WrongDestination(_)
        ));
    }

    #[test]
    fn process_response_rejects_non_success_status() {
        let b = SamlBroker::new();
        let (_url, req_id) = b
            .start_sp_initiated_login(
                "https://sp.cave",
                "https://idp.example/sso",
                "https://idp.example",
                "https://sp.cave/acs",
            )
            .unwrap();
        let a = Assertion::new("https://idp.example", "alice");
        let mut r = Response::success(
            "https://idp.example",
            "https://sp.cave/acs",
            Some(req_id),
            a,
        );
        r.status = StatusCode::Responder;
        assert!(b.process_response(r).is_err());
    }

    #[test]
    fn process_response_rejects_expired_assertion() {
        let b = SamlBroker::new();
        let (_url, req_id) = b
            .start_sp_initiated_login(
                "https://sp.cave",
                "https://idp.example/sso",
                "https://idp.example",
                "https://sp.cave/acs",
            )
            .unwrap();
        let mut a = Assertion::new("https://idp.example", "alice");
        a.not_before = Utc::now() - chrono::Duration::hours(2);
        a.not_on_or_after = Utc::now() - chrono::Duration::hours(1);
        let r = Response::success(
            "https://idp.example",
            "https://sp.cave/acs",
            Some(req_id),
            a,
        );
        assert!(matches!(
            b.process_response(r).unwrap_err(),
            SamlError::Expired
        ));
    }

    #[test]
    fn gc_removes_expired_in_flight() {
        let mut b = SamlBroker::new();
        b.ttl_seconds = 1;
        let _ = b
            .start_sp_initiated_login("sp", "https://idp/sso", "idp", "https://sp/acs")
            .unwrap();
        assert_eq!(b.in_flight_count(), 1);

        // Backdate the entry by two seconds.
        {
            let mut g = b.in_flight.write().unwrap();
            for v in g.values_mut() {
                v.created_at = v.created_at - chrono::Duration::seconds(5);
            }
        }
        b.gc();
        assert_eq!(b.in_flight_count(), 0);
    }

    #[test]
    fn idp_accept_rejects_wrong_destination() {
        let b = SamlBroker::new();
        let req = AuthnRequest::new("https://sp.cave", "https://idp.cave/sso");
        let err = b
            .accept_authn_request(req, "https://idp.cave/different")
            .unwrap_err();
        assert!(matches!(err, SamlError::WrongDestination(_)));
    }

    #[test]
    fn mint_response_carries_in_response_to_and_audiences() {
        let b = SamlBroker::new();
        let req = AuthnRequest::new("https://sp.cave", "https://idp.cave/sso")
            .with_acs_url("https://sp.cave/acs");
        let r = b.mint_response("https://idp.cave", &req, fake_subject());
        assert_eq!(r.in_response_to.as_deref(), Some(req.id.as_str()));
        let a = r.assertion.unwrap();
        assert_eq!(a.audiences, vec!["https://sp.cave".to_string()]);
        assert_eq!(a.subject_name_id, "alice@example.com");
    }

    #[test]
    fn supported_bindings_advertises_post_and_redirect() {
        let b = SamlBroker::new();
        let bs = b.supported_bindings();
        assert!(bs.iter().any(|s| s.contains("HTTP-Redirect")));
        assert!(bs.iter().any(|s| s.contains("HTTP-POST")));
    }
}

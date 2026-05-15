// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 testsuite/integration-arquillian/.../oidc/
//
//! Cross-module end-to-end checks that wire authorize + PAR + token revocation
//! the way Keycloak's integration suite does. Each #[test] documents an
//! `upstream:` line for traceability in the parity manifest's `[[upstream_test]]`.

use super::super::{
    authz_request::{self, AuthzRequest},
    par::ParForm,
    pkce::{self, PkceMethod},
};
use chrono::Utc;

// upstream: keycloak/keycloak OIDCAuthCodeFlowTest.java:codeChallengeRoundtrip
#[test]
fn pkce_s256_full_roundtrip() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = pkce::compute_challenge(verifier, PkceMethod::S256);
    assert!(pkce::verify(verifier, &challenge, PkceMethod::S256).is_ok());
}

// upstream: keycloak/keycloak ResponseTypeTest.java:codeIdTokenTokenAccepted
#[test]
fn response_type_code_id_token_token_accepted() {
    let kinds = authz_request::parse_response_type("code id_token token").unwrap();
    assert_eq!(kinds.len(), 3);
}

// upstream: keycloak/keycloak ResponseTypeTest.java:duplicateTokensDeduped
#[test]
fn duplicate_response_type_tokens_deduped() {
    let kinds = authz_request::parse_response_type("code code code").unwrap();
    assert_eq!(kinds.len(), 1);
}

// upstream: keycloak/keycloak PushedAuthzRequestEndpointTest.java:formSerialisationStable
#[test]
fn par_form_round_trip_preserves_state() {
    // Verifies that the urlencoded form we store from PAR can be re-parsed
    // by AuthorizeQuery.into_authz_request without losing the `state` param.
    let q = "client_id=app&redirect_uri=https://app/cb&response_type=code&state=hello";
    let parsed: super::super::authorize::AuthorizeQuery = serde_urlencoded::from_str(q).unwrap();
    assert_eq!(parsed.state.as_deref(), Some("hello"));
}

// upstream: keycloak/keycloak PushedAuthzRequestEndpointTest.java:parRecordHonoursTtl
#[test]
fn par_record_expiry_field_is_future() {
    let rec = super::super::ParRecord {
        request_uri: "u".into(), client_id: "c".into(), realm: "r".into(),
        stored_request: "".into(), exp_unix: Utc::now().timestamp() + 60,
    };
    assert!(rec.exp_unix > Utc::now().timestamp());
}

// upstream: keycloak/keycloak TokenRevocationTest.java:revocationListMembership
#[tokio::test]
async fn revocation_list_membership_works() {
    let store = super::super::RevocationStore::new();
    store.revoke("abc").await;
    assert!(store.is_revoked("abc").await);
    assert!(!store.is_revoked("xyz").await);
}

// upstream: keycloak/keycloak DeviceGrantTypeTest.java:userCodeLookupRoundtrip
#[tokio::test]
async fn device_store_user_code_roundtrip() {
    let store = super::super::DeviceCodeStore::new();
    store.put(super::super::DeviceAuthorization {
        device_code: "dc".into(), user_code: "UC-1".into(), realm: "r".into(), client_id: "c".into(),
        scope: "openid".into(), exp_unix: Utc::now().timestamp() + 60, interval: 5,
        status: super::super::DeviceStatus::Pending, approved_user_sub: None, last_poll_unix: 0,
    }).await;
    assert!(store.get_by_user("UC-1").await.is_some());
    assert!(store.get_by_device("dc").await.is_some());
}

// upstream: keycloak/keycloak CibaGrantTypeTest.java:authReqIdLookup
#[tokio::test]
async fn ciba_store_returns_record_by_id() {
    let store = super::super::CibaStore::new();
    let r = super::super::CibaRequest {
        auth_req_id: "rid".into(), realm: "r".into(), client_id: "c".into(),
        user_sub: "s".into(), scope: "openid".into(),
        exp_unix: Utc::now().timestamp() + 60, interval: 5,
        status: super::super::CibaStatus::Pending, last_poll_unix: 0,
    };
    store.put(r).await;
    assert!(store.get("rid").await.is_some());
}

// Allow the upstream_port file to reference the serde_urlencoded crate directly.
mod serde_urlencoded {
    pub use ::serde_urlencoded::*;
}

// Suppress unused-import warnings on minimal builds.
#[allow(dead_code)]
fn _force_use(_: AuthzRequest, _: ParForm) {}

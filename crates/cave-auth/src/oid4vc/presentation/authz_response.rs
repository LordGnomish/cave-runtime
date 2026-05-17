// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/OID4VPVerifierEndpoint.java

//! OID4VP authorization response — RED phase.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use super::super::issuance::credential_endpoint::IssuerState;
use super::super::vc::model::VerifiableCredential;
use super::super::Oid4vcError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzResponse {
    pub vp_token: VpToken,
    #[serde(default)]
    pub presentation_submission: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VpToken {
    Single(VerifiableCredential),
    Multi(Vec<VerifiableCredential>),
}

impl VpToken {
    pub fn credentials(&self) -> Vec<&VerifiableCredential> {
        match self {
            VpToken::Single(c) => vec![c],
            VpToken::Multi(v) => v.iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzResponseResult {
    pub verified: bool,
    pub state: Option<String>,
    pub subject_ids: Vec<String>,
}

pub async fn handle_authz_response(
    State(_st): State<IssuerState>,
    Json(_body): Json<AuthzResponse>,
) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "RED-phase stub").into_response()
}

pub fn verify_vp(_body: &AuthzResponse, _pk: &VerifyingKey) -> Result<AuthzResponseResult, Oid4vcError> {
    Err(Oid4vcError::Parse("RED-phase stub".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oid4vc::issuance::credential_endpoint::{IssueRequest, IssuanceTicket, IssuerKeys, IssuerState};
    use serde_json::json;

    fn issued(st: &IssuerState) -> VerifiableCredential {
        st.add_pre_auth_code("c", IssuanceTicket {
            subject_did: "did:example:alice".into(),
            claims: { let mut m = serde_json::Map::new(); m.insert("name".into(), json!("Alice")); m },
            credential_type: "EmployeeCredential".into(),
            used: false,
        });
        let req = IssueRequest { credential_configuration_id: None, pre_authorized_code: Some("c".into()) };
        crate::oid4vc::issuance::credential_endpoint::issue(st, &axum::http::HeaderMap::new(), &req).unwrap()
    }

    #[test]
    fn vp_token_single_accepts_one_credential() {
        let st = IssuerState::new(IssuerKeys::test());
        let vc = issued(&st);
        let body = AuthzResponse { vp_token: VpToken::Single(vc), presentation_submission: serde_json::Value::Null, state: Some("xyz".into()) };
        let pk = st.inner.read().keys.signing_key.verifying_key();
        let r = verify_vp(&body, &pk).unwrap();
        assert!(r.verified);
        assert_eq!(r.subject_ids, vec!["did:example:alice".to_string()]);
        assert_eq!(r.state.as_deref(), Some("xyz"));
    }

    #[test]
    fn vp_token_multi_verifies_every_credential() {
        let st = IssuerState::new(IssuerKeys::test());
        let vc1 = issued(&st);
        st.add_pre_auth_code("c2", IssuanceTicket {
            subject_did: "did:example:bob".into(),
            claims: serde_json::Map::new(),
            credential_type: "EmployeeCredential".into(),
            used: false,
        });
        let req = IssueRequest { credential_configuration_id: None, pre_authorized_code: Some("c2".into()) };
        let vc2 = crate::oid4vc::issuance::credential_endpoint::issue(&st, &axum::http::HeaderMap::new(), &req).unwrap();
        let body = AuthzResponse { vp_token: VpToken::Multi(vec![vc1, vc2]), presentation_submission: serde_json::Value::Null, state: None };
        let pk = st.inner.read().keys.signing_key.verifying_key();
        let r = verify_vp(&body, &pk).unwrap();
        assert_eq!(r.subject_ids.len(), 2);
    }

    #[test]
    fn vp_token_with_tampered_credential_rejected() {
        let st = IssuerState::new(IssuerKeys::test());
        let mut vc = issued(&st);
        vc.credential_subject.claims.insert("name".into(), json!("Mallory"));
        let body = AuthzResponse { vp_token: VpToken::Single(vc), presentation_submission: serde_json::Value::Null, state: None };
        let pk = st.inner.read().keys.signing_key.verifying_key();
        let err = verify_vp(&body, &pk).unwrap_err();
        assert!(matches!(err, Oid4vcError::Signature(_)));
    }

    #[test]
    fn vp_token_empty_multi_rejected() {
        let st = IssuerState::new(IssuerKeys::test());
        let body = AuthzResponse { vp_token: VpToken::Multi(vec![]), presentation_submission: serde_json::Value::Null, state: None };
        let pk = st.inner.read().keys.signing_key.verifying_key();
        let err = verify_vp(&body, &pk).unwrap_err();
        assert!(matches!(err, Oid4vcError::MissingField(_)));
    }

    #[tokio::test]
    async fn handler_returns_200_for_valid_vp() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let st = IssuerState::new(IssuerKeys::test());
        let vc = issued(&st);
        let app = axum::Router::new().route("/oid4vp/authz_response", axum::routing::post(handle_authz_response)).with_state(st);
        let body = serde_json::to_string(&AuthzResponse { vp_token: VpToken::Single(vc), presentation_submission: serde_json::Value::Null, state: Some("xyz".into()) }).unwrap();
        let req = Request::builder().uri("/oid4vp/authz_response").method("POST").header("content-type", "application/json").body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let r: AuthzResponseResult = serde_json::from_slice(&body).unwrap();
        assert!(r.verified);
        assert_eq!(r.state.as_deref(), Some("xyz"));
    }

    #[tokio::test]
    async fn handler_returns_400_for_tampered_vp() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let st = IssuerState::new(IssuerKeys::test());
        let mut vc = issued(&st);
        vc.credential_subject.claims.insert("name".into(), json!("Mallory"));
        let app = axum::Router::new().route("/oid4vp/authz_response", axum::routing::post(handle_authz_response)).with_state(st);
        let body = serde_json::to_string(&AuthzResponse { vp_token: VpToken::Single(vc), presentation_submission: serde_json::Value::Null, state: None }).unwrap();
        let req = Request::builder().uri("/oid4vp/authz_response").method("POST").header("content-type", "application/json").body(Body::from(body)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

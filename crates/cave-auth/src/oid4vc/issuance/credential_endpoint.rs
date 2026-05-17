// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/OID4VCIssuerEndpoint.java

//! Credential endpoint — RED phase.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::State, http::{HeaderMap, StatusCode}, response::IntoResponse, Json};
use ed25519_dalek::SigningKey;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::credential_offer::CredentialOffer;
use super::super::vc::model::VerifiableCredential;
use super::super::Oid4vcError;

#[derive(Clone)]
pub struct IssuerState {
    pub inner: Arc<RwLock<IssuerStateInner>>,
}

pub struct IssuerStateInner {
    pub current_offer: Option<CredentialOffer>,
    pub pre_auth_codes: HashMap<String, IssuanceTicket>,
    pub bearer_tokens: HashMap<String, IssuanceTicket>,
    pub keys: IssuerKeys,
}

pub struct IssuerKeys {
    pub signing_key: SigningKey,
    pub verification_method: String,
    pub issuer_did: String,
}

impl IssuerKeys {
    pub fn test() -> Self {
        let seed = [42u8; 32];
        Self {
            signing_key: SigningKey::from_bytes(&seed),
            verification_method: "did:example:test-issuer#key-1".into(),
            issuer_did: "did:example:test-issuer".into(),
        }
    }
}

#[derive(Clone)]
pub struct IssuanceTicket {
    pub subject_did: String,
    pub claims: serde_json::Map<String, serde_json::Value>,
    pub credential_type: String,
    pub used: bool,
}

impl IssuerState {
    pub fn new(keys: IssuerKeys) -> Self {
        Self {
            inner: Arc::new(RwLock::new(IssuerStateInner {
                current_offer: None,
                pre_auth_codes: HashMap::new(),
                bearer_tokens: HashMap::new(),
                keys,
            })),
        }
    }
    pub fn add_pre_auth_code(&self, code: impl Into<String>, ticket: IssuanceTicket) {
        self.inner.write().pre_auth_codes.insert(code.into(), ticket);
    }
    pub fn add_bearer_token(&self, token: impl Into<String>, ticket: IssuanceTicket) {
        self.inner.write().bearer_tokens.insert(token.into(), ticket);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRequest {
    #[serde(default)]
    pub credential_configuration_id: Option<String>,
    #[serde(default, rename = "pre-authorized_code")]
    pub pre_authorized_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueResponse {
    pub credential: VerifiableCredential,
}

pub async fn handle_issue(
    State(_st): State<IssuerState>,
    _headers: HeaderMap,
    Json(_req): Json<IssueRequest>,
) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "RED-phase stub").into_response()
}

pub fn issue(
    _st: &IssuerState,
    _headers: &HeaderMap,
    _req: &IssueRequest,
) -> Result<VerifiableCredential, Oid4vcError> {
    Err(Oid4vcError::Parse("RED-phase stub".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ticket() -> IssuanceTicket {
        IssuanceTicket {
            subject_did: "did:example:alice".into(),
            claims: { let mut m = serde_json::Map::new(); m.insert("name".into(), json!("Alice")); m },
            credential_type: "EmployeeCredential".into(),
            used: false,
        }
    }

    #[test]
    fn issue_via_pre_auth_code_returns_signed_credential() {
        let st = IssuerState::new(IssuerKeys::test());
        st.add_pre_auth_code("code-abc", ticket());
        let req = IssueRequest { credential_configuration_id: Some("EmployeeCredential".into()), pre_authorized_code: Some("code-abc".into()) };
        let vc = issue(&st, &HeaderMap::new(), &req).unwrap();
        assert!(vc.proof.is_some());
        assert_eq!(vc.credential_subject.id.as_deref(), Some("did:example:alice"));
        assert!(vc.credential_type.contains(&"EmployeeCredential".to_string()));
    }

    #[test]
    fn pre_auth_code_is_single_use() {
        let st = IssuerState::new(IssuerKeys::test());
        st.add_pre_auth_code("code-1", ticket());
        let req = IssueRequest { credential_configuration_id: Some("EmployeeCredential".into()), pre_authorized_code: Some("code-1".into()) };
        assert!(issue(&st, &HeaderMap::new(), &req).is_ok());
        let err = issue(&st, &HeaderMap::new(), &req).unwrap_err();
        assert!(matches!(err, Oid4vcError::InvalidGrant(_)));
    }

    #[test]
    fn unknown_pre_auth_code_rejected() {
        let st = IssuerState::new(IssuerKeys::test());
        let req = IssueRequest { credential_configuration_id: Some("EmployeeCredential".into()), pre_authorized_code: Some("nope".into()) };
        let err = issue(&st, &HeaderMap::new(), &req).unwrap_err();
        assert!(matches!(err, Oid4vcError::InvalidGrant(_)));
    }

    #[test]
    fn bearer_token_grant_issues_credential() {
        let st = IssuerState::new(IssuerKeys::test());
        st.add_bearer_token("tok-1", ticket());
        let req = IssueRequest { credential_configuration_id: None, pre_authorized_code: None };
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer tok-1".parse().unwrap());
        let vc = issue(&st, &h, &req).unwrap();
        assert!(vc.proof.is_some());
    }

    #[test]
    fn bearer_token_is_single_use() {
        let st = IssuerState::new(IssuerKeys::test());
        st.add_bearer_token("tok-2", ticket());
        let req = IssueRequest { credential_configuration_id: None, pre_authorized_code: None };
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer tok-2".parse().unwrap());
        assert!(issue(&st, &h, &req).is_ok());
        assert!(matches!(issue(&st, &h, &req).unwrap_err(), Oid4vcError::InvalidGrant(_)));
    }

    #[test]
    fn missing_grant_and_token_rejected() {
        let st = IssuerState::new(IssuerKeys::test());
        let req = IssueRequest { credential_configuration_id: None, pre_authorized_code: None };
        let err = issue(&st, &HeaderMap::new(), &req).unwrap_err();
        assert!(matches!(err, Oid4vcError::MissingField(_)));
    }

    #[test]
    fn issued_credential_is_verifiable() {
        let st = IssuerState::new(IssuerKeys::test());
        st.add_pre_auth_code("c", ticket());
        let req = IssueRequest { credential_configuration_id: None, pre_authorized_code: Some("c".into()) };
        let vc = issue(&st, &HeaderMap::new(), &req).unwrap();
        let pk = st.inner.read().keys.signing_key.verifying_key();
        crate::oid4vc::vc::proof::verify_credential(&vc, &pk).unwrap();
    }

    #[tokio::test]
    async fn handler_returns_400_for_unknown_code() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let st = IssuerState::new(IssuerKeys::test());
        let app = axum::Router::new().route("/credential", axum::routing::post(handle_issue)).with_state(st);
        let req = Request::builder().uri("/credential").method("POST").header("content-type", "application/json").body(Body::from(r#"{"pre-authorized_code":"nope"}"#)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_issues_via_pre_auth_code_e2e() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let st = IssuerState::new(IssuerKeys::test());
        st.add_pre_auth_code("good-code", ticket());
        let app = axum::Router::new().route("/credential", axum::routing::post(handle_issue)).with_state(st);
        let req = Request::builder().uri("/credential").method("POST").header("content-type", "application/json").body(Body::from(r#"{"pre-authorized_code":"good-code"}"#)).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let r: IssueResponse = serde_json::from_slice(&body).unwrap();
        assert!(r.credential.proof.is_some());
    }
}

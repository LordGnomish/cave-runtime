// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/OID4VCIssuerEndpoint.java

//! OID4VCI credential endpoint — `POST /oid4vc/credential`.
//!
//! Accepts either a pre-authorized-code grant (the wallet redeems the code
//! it pulled out of a credential offer) or an OAuth authorization-code
//! access token. We model both via [`IssueRequest`] (with optional
//! `pre-authorized_code`) and an `Authorization: Bearer …` header.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use ed25519_dalek::SigningKey;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::credential_offer::CredentialOffer;
use super::super::vc::model::{CredentialSubject, VerifiableCredential};
use super::super::vc::proof::sign_credential;
use super::super::Oid4vcError;

/// Issuer-side state shared across the OID4VC routes.
#[derive(Clone)]
pub struct IssuerState {
    /// Inner mutable state — wrapped in `Arc<RwLock<…>>` so axum can clone
    /// the handle into every request.
    pub inner: Arc<RwLock<IssuerStateInner>>,
}

/// Mutable parts of the issuer state.
pub struct IssuerStateInner {
    /// Optional current credential offer (served by [`super::credential_offer`]).
    pub current_offer: Option<CredentialOffer>,
    /// Single-use pre-authorized codes mapped to the subject claims to
    /// issue. Real deployments persist this in cave-store.
    pub pre_auth_codes: HashMap<String, IssuanceTicket>,
    /// Valid bearer tokens (authorization-code grant) mapped to the same.
    pub bearer_tokens: HashMap<String, IssuanceTicket>,
    /// Issuer keys for VC signing.
    pub keys: IssuerKeys,
}

/// Issuer signing material.
pub struct IssuerKeys {
    /// Ed25519 signer for VC DataIntegrityProof.
    pub signing_key: SigningKey,
    /// DID URL the verifier resolves to find the matching public key.
    pub verification_method: String,
    /// Issuer DID — value of `issuer` on emitted credentials.
    pub issuer_did: String,
}

impl IssuerKeys {
    /// Deterministic test keypair — DO NOT use outside tests.
    pub fn test() -> Self {
        // Fixed seed for reproducibility.
        let seed = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        Self {
            signing_key,
            verification_method: "did:example:test-issuer#key-1".into(),
            issuer_did: "did:example:test-issuer".into(),
        }
    }
}

/// What an unredeemed pre-authorized code / bearer token gets exchanged for.
#[derive(Clone)]
pub struct IssuanceTicket {
    /// Subject DID for `credentialSubject.id`.
    pub subject_did: String,
    /// Subject claims to include in the credential.
    pub claims: serde_json::Map<String, serde_json::Value>,
    /// Credential configuration / type to issue (e.g. `"EmployeeCredential"`).
    pub credential_type: String,
    /// Whether this code/token has been used (single-use).
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

    /// Register a single-use pre-authorized code.
    pub fn add_pre_auth_code(&self, code: impl Into<String>, ticket: IssuanceTicket) {
        self.inner
            .write()
            .pre_auth_codes
            .insert(code.into(), ticket);
    }

    /// Register a bearer token.
    pub fn add_bearer_token(&self, token: impl Into<String>, ticket: IssuanceTicket) {
        self.inner
            .write()
            .bearer_tokens
            .insert(token.into(), ticket);
    }
}

/// `POST /credential` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRequest {
    /// Credential identifier the wallet is requesting.
    #[serde(default)]
    pub credential_configuration_id: Option<String>,
    /// Pre-authorized-code grant (alternative to bearer auth).
    #[serde(default, rename = "pre-authorized_code")]
    pub pre_authorized_code: Option<String>,
}

/// `POST /credential` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueResponse {
    /// `credential` — the signed VC as a JSON object.
    pub credential: VerifiableCredential,
}

/// Axum handler.
pub async fn handle_issue(
    State(st): State<IssuerState>,
    headers: HeaderMap,
    Json(req): Json<IssueRequest>,
) -> axum::response::Response {
    match issue(&st, &headers, &req) {
        Ok(vc) => (StatusCode::OK, Json(IssueResponse { credential: vc })).into_response(),
        Err(Oid4vcError::InvalidGrant(m)) => (StatusCode::BAD_REQUEST, format!("invalid_grant: {m}")).into_response(),
        Err(Oid4vcError::MissingField(m)) => (StatusCode::BAD_REQUEST, format!("missing: {m}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

/// Pure issuance logic (no axum types) — easy to test directly.
pub fn issue(
    st: &IssuerState,
    headers: &HeaderMap,
    req: &IssueRequest,
) -> Result<VerifiableCredential, Oid4vcError> {
    // Resolve the issuance ticket via either pre-auth code or bearer token.
    let ticket = if let Some(code) = req.pre_authorized_code.as_ref() {
        let mut inner = st.inner.write();
        let t = inner
            .pre_auth_codes
            .get(code)
            .cloned()
            .ok_or_else(|| Oid4vcError::InvalidGrant("pre-auth code not found".into()))?;
        if t.used {
            return Err(Oid4vcError::InvalidGrant("pre-auth code already used".into()));
        }
        // Mark used.
        if let Some(stored) = inner.pre_auth_codes.get_mut(code) {
            stored.used = true;
        }
        t
    } else if let Some(auth) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
        let token = auth.strip_prefix("Bearer ").unwrap_or(auth);
        let mut inner = st.inner.write();
        let t = inner
            .bearer_tokens
            .get(token)
            .cloned()
            .ok_or_else(|| Oid4vcError::InvalidGrant("bearer token not found".into()))?;
        if t.used {
            return Err(Oid4vcError::InvalidGrant("bearer token already used".into()));
        }
        if let Some(stored) = inner.bearer_tokens.get_mut(token) {
            stored.used = true;
        }
        t
    } else {
        return Err(Oid4vcError::MissingField("grant or bearer".into()));
    };

    // Construct + sign the credential.
    let mut subject = CredentialSubject::new().with_id(&ticket.subject_did);
    subject.claims = ticket.claims;
    let inner = st.inner.read();
    let vc = VerifiableCredential::new(&inner.keys.issuer_did, subject)
        .with_type(&ticket.credential_type);
    sign_credential(
        vc,
        &inner.keys.signing_key,
        inner.keys.verification_method.clone(),
        chrono::Utc::now(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ticket() -> IssuanceTicket {
        IssuanceTicket {
            subject_did: "did:example:alice".into(),
            claims: {
                let mut m = serde_json::Map::new();
                m.insert("name".into(), json!("Alice"));
                m
            },
            credential_type: "EmployeeCredential".into(),
            used: false,
        }
    }

    #[test]
    fn issue_via_pre_auth_code_returns_signed_credential() {
        let st = IssuerState::new(IssuerKeys::test());
        st.add_pre_auth_code("code-abc", ticket());
        let req = IssueRequest {
            credential_configuration_id: Some("EmployeeCredential".into()),
            pre_authorized_code: Some("code-abc".into()),
        };
        let vc = issue(&st, &HeaderMap::new(), &req).unwrap();
        assert!(vc.proof.is_some());
        assert_eq!(vc.credential_subject.id.as_deref(), Some("did:example:alice"));
        assert!(vc.credential_type.contains(&"EmployeeCredential".to_string()));
    }

    #[test]
    fn pre_auth_code_is_single_use() {
        let st = IssuerState::new(IssuerKeys::test());
        st.add_pre_auth_code("code-1", ticket());
        let req = IssueRequest {
            credential_configuration_id: Some("EmployeeCredential".into()),
            pre_authorized_code: Some("code-1".into()),
        };
        assert!(issue(&st, &HeaderMap::new(), &req).is_ok());
        let err = issue(&st, &HeaderMap::new(), &req).unwrap_err();
        assert!(matches!(err, Oid4vcError::InvalidGrant(_)));
    }

    #[test]
    fn unknown_pre_auth_code_rejected() {
        let st = IssuerState::new(IssuerKeys::test());
        let req = IssueRequest {
            credential_configuration_id: Some("EmployeeCredential".into()),
            pre_authorized_code: Some("nope".into()),
        };
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
        assert!(matches!(
            issue(&st, &h, &req).unwrap_err(),
            Oid4vcError::InvalidGrant(_)
        ));
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
        let req = IssueRequest {
            credential_configuration_id: None,
            pre_authorized_code: Some("c".into()),
        };
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
        let app = axum::Router::new()
            .route("/credential", axum::routing::post(handle_issue))
            .with_state(st);
        let req = Request::builder()
            .uri("/credential")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"pre-authorized_code":"nope"}"#))
            .unwrap();
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
        let app = axum::Router::new()
            .route("/credential", axum::routing::post(handle_issue))
            .with_state(st);
        let req = Request::builder()
            .uri("/credential")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"pre-authorized_code":"good-code"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let r: IssueResponse = serde_json::from_slice(&body).unwrap();
        assert!(r.credential.proof.is_some());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/OID4VPVerifierEndpoint.java

//! OID4VP Authorization Response — the wallet POSTs back a VP token.
//!
//! `POST /oid4vp/authz_response` body:
//! ```json
//! {
//!   "vp_token": "<JSON Verifiable Presentation>",
//!   "presentation_submission": { ... },
//!   "state": "..."
//! }
//! ```
//!
//! We verify the VP signature and return either 200 (with the matched
//! credentials) or 400 with a structured error.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use super::super::Oid4vcError;
use super::super::issuance::credential_endpoint::IssuerState;
use super::super::vc::model::VerifiableCredential;
use super::super::vc::proof::verify_credential;

/// Wire shape of the wallet's POST body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzResponse {
    /// `vp_token` — single VC for simple flows; arrays accepted but we
    /// only inspect the first credential here.
    pub vp_token: VpToken,
    /// `presentation_submission` — opaque descriptor, echoed back.
    #[serde(default)]
    pub presentation_submission: serde_json::Value,
    /// `state` — echoed back from the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// The vp_token may be a single VC or an array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VpToken {
    /// Single credential.
    Single(VerifiableCredential),
    /// Multiple credentials.
    Multi(Vec<VerifiableCredential>),
}

impl VpToken {
    /// All credentials in the token (length 1 for `Single`).
    pub fn credentials(&self) -> Vec<&VerifiableCredential> {
        match self {
            VpToken::Single(c) => vec![c],
            VpToken::Multi(v) => v.iter().collect(),
        }
    }
}

/// Successful verification result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzResponseResult {
    pub verified: bool,
    pub state: Option<String>,
    pub subject_ids: Vec<String>,
}

pub async fn handle_authz_response(
    State(st): State<IssuerState>,
    Json(body): Json<AuthzResponse>,
) -> axum::response::Response {
    // For verification, the verifier needs the credential's issuer key —
    // for this demo we use the issuer's own key (the verifier and issuer
    // are co-located). Real deployments resolve the DID and fetch the
    // public key out-of-band.
    let pk = st.inner.read().keys.signing_key.verifying_key();
    match verify_vp(&body, &pk) {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(Oid4vcError::Signature(m)) => {
            (StatusCode::BAD_REQUEST, format!("signature: {m}")).into_response()
        }
        Err(Oid4vcError::MissingField(m)) => {
            (StatusCode::BAD_REQUEST, format!("missing: {m}")).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e}")).into_response(),
    }
}

/// Pure verification logic — verifies every credential in the VP token
/// against `pk` and returns the collected subject IDs.
pub fn verify_vp(
    body: &AuthzResponse,
    pk: &VerifyingKey,
) -> Result<AuthzResponseResult, Oid4vcError> {
    let creds = body.vp_token.credentials();
    if creds.is_empty() {
        return Err(Oid4vcError::MissingField("vp_token credentials".into()));
    }
    let mut subjects = Vec::new();
    for c in &creds {
        verify_credential(c, pk)?;
        if let Some(id) = c.credential_subject.id.clone() {
            subjects.push(id);
        }
    }
    Ok(AuthzResponseResult {
        verified: true,
        state: body.state.clone(),
        subject_ids: subjects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oid4vc::issuance::credential_endpoint::{
        IssuanceTicket, IssueRequest, IssuerKeys, IssuerState,
    };
    use serde_json::json;

    fn issued(st: &IssuerState) -> VerifiableCredential {
        st.add_pre_auth_code(
            "c",
            IssuanceTicket {
                subject_did: "did:example:alice".into(),
                claims: {
                    let mut m = serde_json::Map::new();
                    m.insert("name".into(), json!("Alice"));
                    m
                },
                credential_type: "EmployeeCredential".into(),
                used: false,
            },
        );
        let req = IssueRequest {
            credential_configuration_id: None,
            pre_authorized_code: Some("c".into()),
        };
        crate::oid4vc::issuance::credential_endpoint::issue(st, &axum::http::HeaderMap::new(), &req)
            .unwrap()
    }

    #[test]
    fn vp_token_single_accepts_one_credential() {
        let st = IssuerState::new(IssuerKeys::test());
        let vc = issued(&st);
        let body = AuthzResponse {
            vp_token: VpToken::Single(vc),
            presentation_submission: serde_json::Value::Null,
            state: Some("xyz".into()),
        };
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
        // Re-issue another credential.
        st.add_pre_auth_code(
            "c2",
            IssuanceTicket {
                subject_did: "did:example:bob".into(),
                claims: serde_json::Map::new(),
                credential_type: "EmployeeCredential".into(),
                used: false,
            },
        );
        let req = IssueRequest {
            credential_configuration_id: None,
            pre_authorized_code: Some("c2".into()),
        };
        let vc2 = crate::oid4vc::issuance::credential_endpoint::issue(
            &st,
            &axum::http::HeaderMap::new(),
            &req,
        )
        .unwrap();
        let body = AuthzResponse {
            vp_token: VpToken::Multi(vec![vc1, vc2]),
            presentation_submission: serde_json::Value::Null,
            state: None,
        };
        let pk = st.inner.read().keys.signing_key.verifying_key();
        let r = verify_vp(&body, &pk).unwrap();
        assert_eq!(r.subject_ids.len(), 2);
    }

    #[test]
    fn vp_token_with_tampered_credential_rejected() {
        let st = IssuerState::new(IssuerKeys::test());
        let mut vc = issued(&st);
        vc.credential_subject
            .claims
            .insert("name".into(), json!("Mallory"));
        let body = AuthzResponse {
            vp_token: VpToken::Single(vc),
            presentation_submission: serde_json::Value::Null,
            state: None,
        };
        let pk = st.inner.read().keys.signing_key.verifying_key();
        let err = verify_vp(&body, &pk).unwrap_err();
        assert!(matches!(err, Oid4vcError::Signature(_)));
    }

    #[test]
    fn vp_token_empty_multi_rejected() {
        let st = IssuerState::new(IssuerKeys::test());
        let body = AuthzResponse {
            vp_token: VpToken::Multi(vec![]),
            presentation_submission: serde_json::Value::Null,
            state: None,
        };
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
        let app = axum::Router::new()
            .route(
                "/oid4vp/authz_response",
                axum::routing::post(handle_authz_response),
            )
            .with_state(st);
        let body = serde_json::to_string(&AuthzResponse {
            vp_token: VpToken::Single(vc),
            presentation_submission: serde_json::Value::Null,
            state: Some("xyz".into()),
        })
        .unwrap();
        let req = Request::builder()
            .uri("/oid4vp/authz_response")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
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
        vc.credential_subject
            .claims
            .insert("name".into(), json!("Mallory"));
        let app = axum::Router::new()
            .route(
                "/oid4vp/authz_response",
                axum::routing::post(handle_authz_response),
            )
            .with_state(st);
        let body = serde_json::to_string(&AuthzResponse {
            vp_token: VpToken::Single(vc),
            presentation_submission: serde_json::Value::Null,
            state: None,
        })
        .unwrap();
        let req = Request::builder()
            .uri("/oid4vp/authz_response")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

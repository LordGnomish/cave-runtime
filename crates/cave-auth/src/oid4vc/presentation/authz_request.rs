// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/OID4VPVerifierEndpoint.java

//! OID4VP Authorization Request — the verifier asks a wallet to present
//! credentials matching a `presentation_definition`.
//!
//! `GET /oid4vp/authz?client_id=…&response_type=vp_token&response_mode=direct_post
//!   &presentation_definition=<JSON>&nonce=…&state=…`
//!
//! The wallet performs subject consent and POSTs back to
//! `/oid4vp/authz_response`.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use super::super::issuance::credential_endpoint::IssuerState;

/// Subset of the DIF Presentation Exchange 2.0 `presentation_definition`
/// shape cave-auth recognises. Real wallets accept a much richer schema —
/// the fields below are the ones AD FS / popular wallets emit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationDefinition {
    /// Stable PD identifier.
    pub id: String,
    /// One or more "I want a credential matching these constraints" entries.
    pub input_descriptors: Vec<InputDescriptor>,
}

/// One input descriptor — names a credential type + constraints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputDescriptor {
    /// Descriptor ID.
    pub id: String,
    /// `name` UI hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Required credential `type` (e.g. `"EmployeeCredential"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<String>,
}

/// Query-string structure for `GET /oid4vp/authz`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzRequest {
    /// OAuth `client_id`.
    pub client_id: String,
    /// Always `"vp_token"`.
    pub response_type: String,
    /// `"direct_post"` for the simple browser flow.
    pub response_mode: String,
    /// JSON-encoded `presentation_definition`.
    pub presentation_definition: String,
    /// `nonce` echoed back in the `vp_token`.
    pub nonce: String,
    /// Optional `state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl AuthzRequest {
    /// Parse the embedded `presentation_definition` JSON.
    pub fn parse_definition(&self) -> Result<PresentationDefinition, super::super::Oid4vcError> {
        serde_json::from_str(&self.presentation_definition)
            .map_err(|e| super::super::Oid4vcError::Parse(format!("presentation_definition: {e}")))
    }
}

/// Returned to the wallet — a JSON view of the parsed request the wallet
/// can render and then submit a presentation response for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzRequestView {
    pub client_id: String,
    pub response_type: String,
    pub response_mode: String,
    pub presentation_definition: PresentationDefinition,
    pub nonce: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

pub async fn handle_authz_request(
    State(_st): State<IssuerState>,
    Query(q): Query<BTreeMap<String, String>>,
) -> axum::response::Response {
    let req = match build_authz_request_from_query(&q) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("oid4vp: {e}")).into_response(),
    };
    let def = match req.parse_definition() {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("oid4vp: {e}")).into_response(),
    };
    if req.response_type != "vp_token" {
        return (StatusCode::BAD_REQUEST, "unsupported response_type").into_response();
    }
    let view = AuthzRequestView {
        client_id: req.client_id,
        response_type: req.response_type,
        response_mode: req.response_mode,
        presentation_definition: def,
        nonce: req.nonce,
        state: req.state,
    };
    (StatusCode::OK, Json(view)).into_response()
}

/// Build an `AuthzRequest` from a flat query map. Public to keep the
/// pure logic testable without spinning up axum.
pub fn build_authz_request_from_query(
    q: &BTreeMap<String, String>,
) -> Result<AuthzRequest, super::super::Oid4vcError> {
    fn need<'a>(
        q: &'a BTreeMap<String, String>,
        k: &str,
    ) -> Result<&'a String, super::super::Oid4vcError> {
        q.get(k)
            .ok_or_else(|| super::super::Oid4vcError::MissingField(k.into()))
    }
    Ok(AuthzRequest {
        client_id: need(q, "client_id")?.clone(),
        response_type: need(q, "response_type")?.clone(),
        response_mode: q
            .get("response_mode")
            .cloned()
            .unwrap_or_else(|| "direct_post".into()),
        presentation_definition: need(q, "presentation_definition")?.clone(),
        nonce: need(q, "nonce")?.clone(),
        state: q.get("state").cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oid4vc::issuance::credential_endpoint::{IssuerKeys, IssuerState};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn pd_json() -> String {
        serde_json::to_string(&PresentationDefinition {
            id: "pd-1".into(),
            input_descriptors: vec![InputDescriptor {
                id: "id-1".into(),
                name: Some("Employee".into()),
                credential_type: Some("EmployeeCredential".into()),
            }],
        })
        .unwrap()
    }

    #[test]
    fn presentation_definition_roundtrip_json() {
        let pd = PresentationDefinition {
            id: "pd-1".into(),
            input_descriptors: vec![InputDescriptor {
                id: "id-1".into(),
                name: Some("Employee".into()),
                credential_type: Some("EmployeeCredential".into()),
            }],
        };
        let s = serde_json::to_string(&pd).unwrap();
        let back: PresentationDefinition = serde_json::from_str(&s).unwrap();
        assert_eq!(back, pd);
    }

    #[test]
    fn build_authz_request_extracts_required_fields() {
        let mut q = BTreeMap::new();
        q.insert("client_id".into(), "verifier1".into());
        q.insert("response_type".into(), "vp_token".into());
        q.insert("response_mode".into(), "direct_post".into());
        q.insert("presentation_definition".into(), pd_json());
        q.insert("nonce".into(), "n123".into());
        q.insert("state".into(), "s1".into());
        let r = build_authz_request_from_query(&q).unwrap();
        assert_eq!(r.client_id, "verifier1");
        assert_eq!(r.response_type, "vp_token");
        assert_eq!(r.nonce, "n123");
        assert_eq!(r.state.as_deref(), Some("s1"));
    }

    #[test]
    fn build_authz_request_missing_field_errors() {
        let mut q = BTreeMap::new();
        q.insert("client_id".into(), "v1".into());
        let err = build_authz_request_from_query(&q).unwrap_err();
        assert!(matches!(
            err,
            super::super::super::Oid4vcError::MissingField(_)
        ));
    }

    #[test]
    fn authz_request_parses_embedded_pd() {
        let r = AuthzRequest {
            client_id: "v".into(),
            response_type: "vp_token".into(),
            response_mode: "direct_post".into(),
            presentation_definition: pd_json(),
            nonce: "n".into(),
            state: None,
        };
        let pd = r.parse_definition().unwrap();
        assert_eq!(pd.id, "pd-1");
    }

    #[tokio::test]
    async fn handler_returns_200_with_parsed_pd() {
        let st = IssuerState::new(IssuerKeys::test());
        let app = axum::Router::new()
            .route("/oid4vp/authz", axum::routing::get(handle_authz_request))
            .with_state(st);
        let pd_q = urlencoding::encode(&pd_json()).to_string();
        let uri = format!(
            "/oid4vp/authz?client_id=v1&response_type=vp_token&response_mode=direct_post&presentation_definition={pd_q}&nonce=n42"
        );
        let resp = app
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let v: AuthzRequestView = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.client_id, "v1");
        assert_eq!(v.presentation_definition.id, "pd-1");
    }

    #[tokio::test]
    async fn handler_rejects_non_vp_token_response_type() {
        let st = IssuerState::new(IssuerKeys::test());
        let app = axum::Router::new()
            .route("/oid4vp/authz", axum::routing::get(handle_authz_request))
            .with_state(st);
        let pd_q = urlencoding::encode(&pd_json()).to_string();
        let uri = format!(
            "/oid4vp/authz?client_id=v1&response_type=code&response_mode=direct_post&presentation_definition={pd_q}&nonce=n"
        );
        let resp = app
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_rejects_missing_required_field() {
        let st = IssuerState::new(IssuerKeys::test());
        let app = axum::Router::new()
            .route("/oid4vp/authz", axum::routing::get(handle_authz_request))
            .with_state(st);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/oid4vp/authz?client_id=v1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

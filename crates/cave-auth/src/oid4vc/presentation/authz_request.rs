// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/OID4VPVerifierEndpoint.java

//! OID4VP authorization request — RED phase.

use std::collections::BTreeMap;

use axum::{extract::{Query, State}, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

use super::super::issuance::credential_endpoint::IssuerState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationDefinition {
    pub id: String,
    pub input_descriptors: Vec<InputDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputDescriptor {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzRequest {
    pub client_id: String,
    pub response_type: String,
    pub response_mode: String,
    pub presentation_definition: String,
    pub nonce: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl AuthzRequest {
    pub fn parse_definition(&self) -> Result<PresentationDefinition, super::super::Oid4vcError> {
        Err(super::super::Oid4vcError::Parse("RED-phase stub".into()))
    }
}

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
    Query(_q): Query<BTreeMap<String, String>>,
) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "RED-phase stub").into_response()
}

pub fn build_authz_request_from_query(
    _q: &BTreeMap<String, String>,
) -> Result<AuthzRequest, super::super::Oid4vcError> {
    Err(super::super::Oid4vcError::Parse("RED-phase stub".into()))
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
            input_descriptors: vec![InputDescriptor { id: "id-1".into(), name: Some("Employee".into()), credential_type: Some("EmployeeCredential".into()) }],
        }).unwrap()
    }

    #[test]
    fn presentation_definition_roundtrip_json() {
        let pd = PresentationDefinition {
            id: "pd-1".into(),
            input_descriptors: vec![InputDescriptor { id: "id-1".into(), name: Some("Employee".into()), credential_type: Some("EmployeeCredential".into()) }],
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
        assert!(matches!(err, super::super::super::Oid4vcError::MissingField(_)));
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
        let app = axum::Router::new().route("/oid4vp/authz", axum::routing::get(handle_authz_request)).with_state(st);
        let pd_q = urlencoding::encode(&pd_json()).to_string();
        let uri = format!("/oid4vp/authz?client_id=v1&response_type=vp_token&response_mode=direct_post&presentation_definition={pd_q}&nonce=n42");
        let resp = app.oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let v: AuthzRequestView = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.client_id, "v1");
        assert_eq!(v.presentation_definition.id, "pd-1");
    }

    #[tokio::test]
    async fn handler_rejects_non_vp_token_response_type() {
        let st = IssuerState::new(IssuerKeys::test());
        let app = axum::Router::new().route("/oid4vp/authz", axum::routing::get(handle_authz_request)).with_state(st);
        let pd_q = urlencoding::encode(&pd_json()).to_string();
        let uri = format!("/oid4vp/authz?client_id=v1&response_type=code&response_mode=direct_post&presentation_definition={pd_q}&nonce=n");
        let resp = app.oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_rejects_missing_required_field() {
        let st = IssuerState::new(IssuerKeys::test());
        let app = axum::Router::new().route("/oid4vp/authz", axum::routing::get(handle_authz_request)).with_state(st);
        let resp = app.oneshot(Request::builder().uri("/oid4vp/authz?client_id=v1").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

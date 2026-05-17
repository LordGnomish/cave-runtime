// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/CredentialOfferEndpoint.java

//! Credential Offer — RED phase.

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

use super::credential_endpoint::IssuerState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CredentialOffer {
    pub credential_issuer: String,
    pub credential_configuration_ids: Vec<String>,
    pub grants: CredentialOfferGrants,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CredentialOfferGrants {
    #[serde(rename = "urn:ietf:params:oauth:grant-type:pre-authorized_code",
            skip_serializing_if = "Option::is_none")]
    pub pre_authorized_code: Option<PreAuthorizedCodeGrant>,
    #[serde(rename = "authorization_code", skip_serializing_if = "Option::is_none")]
    pub authorization_code: Option<AuthorizationCodeGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreAuthorizedCodeGrant {
    #[serde(rename = "pre-authorized_code")]
    pub pre_authorized_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_code: Option<TxCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TxCode {
    #[serde(default = "default_tx_type")]
    pub input_mode: String,
    #[serde(default = "default_tx_len")]
    pub length: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
fn default_tx_type() -> String { "RED".into() }
fn default_tx_len() -> u32 { 0 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthorizationCodeGrant {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_server: Option<String>,
}

pub async fn handle_offer(State(_st): State<IssuerState>) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "RED-phase stub").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::credential_endpoint::{IssuerKeys, IssuerState};
    use serde_json::json;

    #[test]
    fn pre_authorized_grant_serialises_with_full_urn_key() {
        let offer = CredentialOffer {
            credential_issuer: "https://issuer.example".into(),
            credential_configuration_ids: vec!["EmployeeCredential".into()],
            grants: CredentialOfferGrants {
                pre_authorized_code: Some(PreAuthorizedCodeGrant {
                    pre_authorized_code: "abc123".into(),
                    tx_code: None,
                }),
                authorization_code: None,
            },
        };
        let j = serde_json::to_value(&offer).unwrap();
        assert!(j["grants"]["urn:ietf:params:oauth:grant-type:pre-authorized_code"]["pre-authorized_code"]
            .as_str().unwrap().contains("abc123"));
    }

    #[test]
    fn tx_code_defaults_to_numeric_length_6() {
        let raw = json!({"pre-authorized_code": "x", "tx_code": {}});
        let g: PreAuthorizedCodeGrant = serde_json::from_value(raw).unwrap();
        let tx = g.tx_code.unwrap();
        assert_eq!(tx.input_mode, "numeric");
        assert_eq!(tx.length, 6);
    }

    #[test]
    fn authorization_code_grant_keyed_as_string_not_urn() {
        let offer = CredentialOffer {
            credential_issuer: "https://issuer.example".into(),
            credential_configuration_ids: vec!["EmployeeCredential".into()],
            grants: CredentialOfferGrants {
                pre_authorized_code: None,
                authorization_code: Some(AuthorizationCodeGrant {
                    issuer_state: Some("state-1".into()),
                    authorization_server: None,
                }),
            },
        };
        let j = serde_json::to_value(&offer).unwrap();
        assert!(j["grants"]["authorization_code"]["issuer_state"].as_str().unwrap().contains("state-1"));
    }

    #[test]
    fn credential_offer_roundtrip_json() {
        let offer = CredentialOffer {
            credential_issuer: "https://issuer.example".into(),
            credential_configuration_ids: vec!["EmployeeCredential".into(), "DriversLicense".into()],
            grants: CredentialOfferGrants {
                pre_authorized_code: Some(PreAuthorizedCodeGrant {
                    pre_authorized_code: "code-xyz".into(),
                    tx_code: Some(TxCode { input_mode: "numeric".into(), length: 4, description: Some("Enter the PIN".into()) }),
                }),
                authorization_code: None,
            },
        };
        let j = serde_json::to_string(&offer).unwrap();
        let back: CredentialOffer = serde_json::from_str(&j).unwrap();
        assert_eq!(back, offer);
    }

    #[tokio::test]
    async fn handler_returns_404_when_no_offer_configured() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let state = IssuerState::new(IssuerKeys::test());
        let app = axum::Router::new()
            .route("/oid4vc/credential_offer", axum::routing::get(handle_offer))
            .with_state(state);
        let resp = app.oneshot(Request::builder().uri("/oid4vc/credential_offer").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handler_returns_configured_offer() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let state = IssuerState::new(IssuerKeys::test());
        let offer = CredentialOffer {
            credential_issuer: "https://i.example".into(),
            credential_configuration_ids: vec!["EmployeeCredential".into()],
            grants: CredentialOfferGrants {
                pre_authorized_code: Some(PreAuthorizedCodeGrant { pre_authorized_code: "code-abc".into(), tx_code: None }),
                authorization_code: None,
            },
        };
        state.inner.write().current_offer = Some(offer.clone());
        let app = axum::Router::new()
            .route("/oid4vc/credential_offer", axum::routing::get(handle_offer))
            .with_state(state);
        let resp = app.oneshot(Request::builder().uri("/oid4vc/credential_offer").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let back: CredentialOffer = serde_json::from_slice(&body).unwrap();
        assert_eq!(back, offer);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/CredentialOfferEndpoint.java

//! OID4VCI Credential Offer — the JSON object an issuer ships to a wallet
//! (out-of-band or via QR code) describing what credentials are on offer
//! and which grant types the wallet can use to redeem them.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

use super::credential_endpoint::IssuerState;

/// Top-level credential-offer object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CredentialOffer {
    /// `credential_issuer` URL.
    pub credential_issuer: String,
    /// `credential_configuration_ids` — IDs the wallet can claim.
    pub credential_configuration_ids: Vec<String>,
    /// `grants` — pre-authorized-code OR authorization-code object.
    pub grants: CredentialOfferGrants,
}

/// Container for the grant entries — both shapes can appear, but
/// every offer must have at least one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CredentialOfferGrants {
    /// Pre-authorized code grant — wallet redeems immediately.
    #[serde(
        rename = "urn:ietf:params:oauth:grant-type:pre-authorized_code",
        skip_serializing_if = "Option::is_none"
    )]
    pub pre_authorized_code: Option<PreAuthorizedCodeGrant>,
    /// Authorization code grant — wallet starts a normal OAuth flow.
    #[serde(rename = "authorization_code", skip_serializing_if = "Option::is_none")]
    pub authorization_code: Option<AuthorizationCodeGrant>,
}

/// Pre-authorized-code grant body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreAuthorizedCodeGrant {
    /// `pre-authorized_code` — single-use credential-issuance code.
    #[serde(rename = "pre-authorized_code")]
    pub pre_authorized_code: String,
    /// `tx_code` — optional transaction code descriptor (e.g. PIN).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_code: Option<TxCode>,
}

/// `tx_code` descriptor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TxCode {
    /// `numeric` | `text`.
    #[serde(default = "default_tx_type")]
    pub input_mode: String,
    /// Length of the code the wallet must request from the user.
    #[serde(default = "default_tx_len")]
    pub length: u32,
    /// `description` — UI hint the wallet can render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_tx_type() -> String {
    "numeric".into()
}
fn default_tx_len() -> u32 {
    6
}

/// Authorization-code grant body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthorizationCodeGrant {
    /// `issuer_state` — opaque correlation token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer_state: Option<String>,
    /// `authorization_server` — optional OP URL override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_server: Option<String>,
}

/// Handler for `GET /oid4vc/credential_offer`. Returns the current offer
/// or 404 if none configured.
pub async fn handle_offer(State(st): State<IssuerState>) -> axum::response::Response {
    let inner = st.inner.read();
    match inner.current_offer.clone() {
        Some(o) => (StatusCode::OK, Json(o)).into_response(),
        None => (StatusCode::NOT_FOUND, "no offer configured").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(j["grants"]
            ["urn:ietf:params:oauth:grant-type:pre-authorized_code"]
            ["pre-authorized_code"]
            .as_str()
            .unwrap()
            .contains("abc123"));
    }

    #[test]
    fn tx_code_defaults_to_numeric_length_6() {
        let raw = json!({
            "pre-authorized_code": "x",
            "tx_code": {}
        });
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
        assert!(
            j["grants"]["authorization_code"]["issuer_state"]
                .as_str()
                .unwrap()
                .contains("state-1")
        );
    }

    #[test]
    fn credential_offer_roundtrip_json() {
        let offer = CredentialOffer {
            credential_issuer: "https://issuer.example".into(),
            credential_configuration_ids: vec![
                "EmployeeCredential".into(),
                "DriversLicense".into(),
            ],
            grants: CredentialOfferGrants {
                pre_authorized_code: Some(PreAuthorizedCodeGrant {
                    pre_authorized_code: "code-xyz".into(),
                    tx_code: Some(TxCode {
                        input_mode: "numeric".into(),
                        length: 4,
                        description: Some("Enter the PIN".into()),
                    }),
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
        let state =
            IssuerState::new(crate::oid4vc::issuance::credential_endpoint::IssuerKeys::test());
        let app = axum::Router::new()
            .route("/oid4vc/credential_offer", axum::routing::get(handle_offer))
            .with_state(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/oid4vc/credential_offer")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handler_returns_configured_offer() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let state =
            IssuerState::new(crate::oid4vc::issuance::credential_endpoint::IssuerKeys::test());
        let offer = CredentialOffer {
            credential_issuer: "https://i.example".into(),
            credential_configuration_ids: vec!["EmployeeCredential".into()],
            grants: CredentialOfferGrants {
                pre_authorized_code: Some(PreAuthorizedCodeGrant {
                    pre_authorized_code: "code-abc".into(),
                    tx_code: None,
                }),
                authorization_code: None,
            },
        };
        state.inner.write().current_offer = Some(offer.clone());
        let app = axum::Router::new()
            .route("/oid4vc/credential_offer", axum::routing::get(handle_offer))
            .with_state(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/oid4vc/credential_offer")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let back: CredentialOffer = serde_json::from_slice(&body).unwrap();
        assert_eq!(back, offer);
    }
}

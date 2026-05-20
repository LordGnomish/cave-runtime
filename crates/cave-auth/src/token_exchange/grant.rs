// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/TokenExchangeGrantType.java + RFC 8693 §2
//
//! `grant_type=urn:ietf:params:oauth:grant-type:token-exchange` request/response.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::actor_token::{ActorClaim, ActorTokenError, parse_actor};
use super::audience_switch::{AudienceError, AudienceRequest};
use super::policy::{ExchangePolicy, PolicyDecision};
use super::subject_token::{SubjectToken, SubjectTokenError, SubjectTokenType};

pub const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExchangeRequest {
    pub grant_type: String,
    pub client_id: String,
    pub subject_token: String,
    pub subject_token_type: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub actor_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub actor_token_type: Option<String>,
    /// `requested_token_type` per RFC 8693 §2.1. Defaults to `access_token`
    /// when omitted (Keycloak behaviour).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub requested_token_type: Option<String>,
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub resource: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
}

/// RFC 8693 §2.2 — successful response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExchangeResponse {
    pub access_token: String,
    pub issued_token_type: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scope: Option<String>,
    /// Convenience: machine-readable claims (audience, sub, act).
    /// Not part of the wire response but useful for downstream signers.
    #[serde(skip)]
    pub claims: ExchangedClaims,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ExchangedClaims {
    pub jti: String,
    pub sub: String,
    pub iss: String,
    pub aud: Vec<String>,
    pub scopes: Vec<String>,
    pub iat: i64,
    pub exp: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub act: Option<ActorClaim>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExchangeError {
    #[error("grant_type must be {0:?}, got {1:?}")]
    BadGrantType(&'static str, String),
    #[error("subject_token + subject_token_type are both required")]
    SubjectMissing,
    #[error("subject token parse failed: {0}")]
    Subject(#[from] SubjectTokenError),
    #[error("actor token parse failed: {0}")]
    Actor(#[from] ActorTokenError),
    #[error("audience invalid: {0}")]
    Audience(#[from] AudienceError),
    #[error("exchange policy denied client {0:?} -> audience {1:?}")]
    PolicyDenied(String, String),
    #[error("requested_token_type is not supported: {0:?}")]
    UnsupportedRequestedType(String),
}

/// The exchanger — wires the parsers + policy + audience into the issuance path.
pub struct TokenExchanger {
    pub issuer: String,
    pub default_lifespan_seconds: i64,
}

impl TokenExchanger {
    pub fn new(issuer: impl Into<String>, lifespan_seconds: i64) -> Self {
        Self {
            issuer: issuer.into(),
            default_lifespan_seconds: lifespan_seconds,
        }
    }

    pub fn exchange(
        &self,
        req: ExchangeRequest,
        policy: &ExchangePolicy,
        now: DateTime<Utc>,
    ) -> Result<ExchangeResponse, ExchangeError> {
        if req.grant_type != GRANT_TYPE {
            return Err(ExchangeError::BadGrantType(GRANT_TYPE, req.grant_type));
        }
        if req.subject_token.is_empty() || req.subject_token_type.is_empty() {
            return Err(ExchangeError::SubjectMissing);
        }

        let subject_type: SubjectTokenType = req
            .subject_token_type
            .parse()
            .map_err(ExchangeError::Subject)?;
        let subject = SubjectToken::parse(&req.subject_token, subject_type)
            .map_err(ExchangeError::Subject)?;

        // RFC 8693 §2.1 — default requested type is access_token.
        let requested_type_str = req
            .requested_token_type
            .clone()
            .unwrap_or_else(|| SubjectTokenType::AccessToken.as_uri().to_string());
        let requested_type: SubjectTokenType = requested_type_str
            .parse()
            .map_err(|_| ExchangeError::UnsupportedRequestedType(requested_type_str.clone()))?;

        // RFC 8693 only mandates support for issuing access_token / jwt / id_token.
        if !matches!(
            requested_type,
            SubjectTokenType::AccessToken | SubjectTokenType::Jwt | SubjectTokenType::IdToken
        ) {
            return Err(ExchangeError::UnsupportedRequestedType(requested_type_str));
        }

        let aud_req = AudienceRequest::new(
            req.audience.clone(),
            req.resource.clone(),
            requested_type,
            req.scope.clone(),
        )
        .map_err(ExchangeError::Audience)?;

        // Policy: every audience must be allowed for this client.
        for aud in &aud_req.audiences {
            if policy.decide(&req.client_id, aud) == PolicyDecision::Deny {
                return Err(ExchangeError::PolicyDenied(
                    req.client_id.clone(),
                    aud.clone(),
                ));
            }
        }

        // Actor token (optional)
        let act = match (req.actor_token.as_deref(), req.actor_token_type.as_deref()) {
            (Some(token), Some(t)) => Some(parse_actor(token, t).map_err(ExchangeError::Actor)?),
            (Some(_), None) => return Err(ExchangeError::Actor(ActorTokenError::TypeRequired)),
            _ => None,
        };

        let lifespan = self.default_lifespan_seconds;
        let claims = ExchangedClaims {
            jti: Uuid::new_v4().to_string(),
            sub: subject.subject.clone(),
            iss: self.issuer.clone(),
            aud: if !aud_req.audiences.is_empty() {
                aud_req.audiences.clone()
            } else {
                aud_req.resources.clone()
            },
            scopes: aud_req.scopes.clone(),
            iat: now.timestamp(),
            exp: (now + Duration::seconds(lifespan)).timestamp(),
            act,
        };

        let access_token = serialise_for_response(&claims);
        Ok(ExchangeResponse {
            access_token,
            issued_token_type: requested_type.as_uri().to_string(),
            token_type: "Bearer".to_string(),
            expires_in: lifespan,
            refresh_token: None,
            scope: if aud_req.scopes.is_empty() {
                None
            } else {
                Some(aud_req.scopes.join(" "))
            },
            claims,
        })
    }
}

/// Produces an opaque (un-signed) access_token blob for the response. The
/// production code path replaces this with the JWS-signed JWT, but we keep
/// the in-tree mint deterministic for tests.
fn serialise_for_response(claims: &ExchangedClaims) -> String {
    use base64::Engine;
    let json = serde_json::to_string(claims).expect("ExchangedClaims is always Serialize");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn b64u(s: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.as_bytes())
    }

    fn jwt(sub: &str) -> String {
        let body = format!(r#"{{"sub":"{sub}","iss":"idp","aud":"rs"}}"#);
        format!(
            "{}.{}.{}",
            b64u(r#"{"alg":"none"}"#),
            b64u(&body),
            b64u("sig")
        )
    }

    fn req() -> ExchangeRequest {
        ExchangeRequest {
            grant_type: GRANT_TYPE.into(),
            client_id: "client-x".into(),
            subject_token: jwt("alice"),
            subject_token_type: SubjectTokenType::AccessToken.as_uri().into(),
            actor_token: None,
            actor_token_type: None,
            requested_token_type: None,
            audience: vec!["billing".into()],
            resource: vec![],
            scope: vec!["read".into()],
        }
    }

    fn allow_billing_for_client_x() -> ExchangePolicy {
        let p = ExchangePolicy::new();
        p.allow("client-x", "billing");
        p
    }

    #[test]
    fn happy_exchange() {
        let ex = TokenExchanger::new("iss", 300);
        let resp = ex
            .exchange(req(), &allow_billing_for_client_x(), Utc::now())
            .unwrap();
        assert_eq!(resp.token_type, "Bearer");
        assert_eq!(
            resp.issued_token_type,
            SubjectTokenType::AccessToken.as_uri()
        );
        assert_eq!(resp.claims.sub, "alice");
    }

    #[test]
    fn wrong_grant_type_rejected() {
        let ex = TokenExchanger::new("iss", 300);
        let mut r = req();
        r.grant_type = "client_credentials".into();
        let err = ex
            .exchange(r, &allow_billing_for_client_x(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, ExchangeError::BadGrantType(_, _)));
    }

    #[test]
    fn missing_subject_token_rejected() {
        let ex = TokenExchanger::new("iss", 300);
        let mut r = req();
        r.subject_token = String::new();
        let err = ex
            .exchange(r, &allow_billing_for_client_x(), Utc::now())
            .unwrap_err();
        assert_eq!(err, ExchangeError::SubjectMissing);
    }

    #[test]
    fn missing_audience_and_resource_rejected() {
        let ex = TokenExchanger::new("iss", 300);
        let mut r = req();
        r.audience.clear();
        r.resource.clear();
        let err = ex
            .exchange(r, &allow_billing_for_client_x(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, ExchangeError::Audience(_)));
    }

    #[test]
    fn policy_denial_blocks() {
        let ex = TokenExchanger::new("iss", 300);
        let policy = ExchangePolicy::new(); // no grants
        let err = ex.exchange(req(), &policy, Utc::now()).unwrap_err();
        assert!(matches!(err, ExchangeError::PolicyDenied(_, _)));
    }

    #[test]
    fn requested_id_token_supported() {
        let ex = TokenExchanger::new("iss", 300);
        let mut r = req();
        r.requested_token_type = Some(SubjectTokenType::IdToken.as_uri().into());
        let resp = ex
            .exchange(r, &allow_billing_for_client_x(), Utc::now())
            .unwrap();
        assert_eq!(resp.issued_token_type, SubjectTokenType::IdToken.as_uri());
    }

    #[test]
    fn unsupported_requested_type_refused() {
        let ex = TokenExchanger::new("iss", 300);
        let mut r = req();
        r.requested_token_type = Some(SubjectTokenType::Saml2.as_uri().into());
        let err = ex
            .exchange(r, &allow_billing_for_client_x(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, ExchangeError::UnsupportedRequestedType(_)));
    }

    #[test]
    fn actor_token_added_to_claims() {
        let ex = TokenExchanger::new("iss", 300);
        let mut r = req();
        r.actor_token = Some(jwt("service-acct"));
        r.actor_token_type = Some(SubjectTokenType::AccessToken.as_uri().into());
        let resp = ex
            .exchange(r, &allow_billing_for_client_x(), Utc::now())
            .unwrap();
        let act = resp.claims.act.expect("act claim missing");
        assert_eq!(act.sub, "service-acct");
    }

    #[test]
    fn actor_token_without_type_rejected() {
        let ex = TokenExchanger::new("iss", 300);
        let mut r = req();
        r.actor_token = Some(jwt("svc"));
        let err = ex
            .exchange(r, &allow_billing_for_client_x(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, ExchangeError::Actor(_)));
    }

    #[test]
    fn multi_audience_each_must_be_allowed() {
        let ex = TokenExchanger::new("iss", 300);
        let p = ExchangePolicy::new();
        p.allow("client-x", "a");
        // not allowing "b" → should fail
        let mut r = req();
        r.audience = vec!["a".into(), "b".into()];
        let err = ex.exchange(r, &p, Utc::now()).unwrap_err();
        assert!(matches!(err, ExchangeError::PolicyDenied(_, _)));
    }

    #[test]
    fn resource_only_uses_resource_as_aud() {
        let ex = TokenExchanger::new("iss", 300);
        let p = ExchangePolicy::new();
        p.allow_any_audience("client-x");
        let mut r = req();
        r.audience.clear();
        r.resource = vec!["https://api/cave".into()];
        let resp = ex.exchange(r, &p, Utc::now()).unwrap();
        assert_eq!(resp.claims.aud, vec!["https://api/cave".to_string()]);
    }

    #[test]
    fn exp_is_iat_plus_lifespan() {
        let ex = TokenExchanger::new("iss", 600);
        let resp = ex
            .exchange(req(), &allow_billing_for_client_x(), Utc::now())
            .unwrap();
        assert_eq!(resp.claims.exp - resp.claims.iat, 600);
    }
}

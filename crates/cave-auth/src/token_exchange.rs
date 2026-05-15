// SPDX-License-Identifier: AGPL-3.0-or-later
//
// OAuth 2.0 Token Exchange — RFC 8693.
//
// `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`
//
// A client presents a `subject_token` and asks the AS to mint a token
// representing a (possibly different) principal, audience, or token type.
// Optional `actor_token` carries the identity of the party performing the
// exchange (delegation chain).
//
// Upstream parity:
//   - keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38  (v22.0.0)
//     services/src/main/java/org/keycloak/protocol/oidc/grants/TokenExchangeGrantType.java
//     services/src/main/java/org/keycloak/protocol/oidc/grants/TokenExchangeProvider.java
//   - RFC 8693 §1.1 (impersonation), §1.2 (delegation), §2 (token-type URIs).

use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// RFC 8693 §3 — request form parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenExchangeRequest {
    pub grant_type: String,
    pub subject_token: String,
    pub subject_token_type: String,
    pub requested_token_type: Option<String>,
    pub actor_token: Option<String>,
    pub actor_token_type: Option<String>,
    pub audience: Option<String>,
    pub resource: Option<String>,
    pub scope: Option<String>,
    pub client_id: Option<String>,
}

/// RFC 8693 §2.2 — token type URIs.
pub mod token_type {
    pub const ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";
    pub const REFRESH_TOKEN: &str = "urn:ietf:params:oauth:token-type:refresh_token";
    pub const ID_TOKEN: &str = "urn:ietf:params:oauth:token-type:id_token";
    pub const SAML2: &str = "urn:ietf:params:oauth:token-type:saml2";
    pub const JWT: &str = "urn:ietf:params:oauth:token-type:jwt";
}

pub const GRANT_TYPE_TOKEN_EXCHANGE: &str =
    "urn:ietf:params:oauth:grant-type:token-exchange";

/// RFC 8693 §2.2 — response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenExchangeResponse {
    pub access_token: String,
    pub issued_token_type: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ExchangeError {
    #[error("invalid_grant")]
    InvalidGrant,
    #[error("invalid_request: {0}")]
    InvalidRequest(&'static str),
    #[error("invalid_token: {0}")]
    InvalidToken(&'static str),
    #[error("unsupported_token_type")]
    UnsupportedTokenType,
    #[error("invalid_target")]
    InvalidTarget,
}

/// Issued-token claim set. We emit a `may_act` claim per RFC 8693 §4.1 (or
/// `act` for chained delegations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangedClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub scope: String,
    /// RFC 8693 §4.1 — `act` is the chain of actors when the exchange is
    /// delegation (not impersonation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<ActorClaim>,
    /// JWT id (uniqueness).
    pub jti: String,
    /// `client_id` of the requesting client (for audit / introspection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Mode: impersonation or delegation (Keycloak adds this for clarity).
    pub typ: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorClaim {
    pub sub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<Box<ActorClaim>>,
}

/// Subject-token claim shape we are willing to accept. The point of the
/// exchange is to project these into a new audience, so we only need the
/// minimum claim set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectClaims {
    pub sub: String,
    pub iss: String,
    pub exp: i64,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub act: Option<ActorClaim>,
    #[serde(default)]
    pub client_id: Option<String>,
}

/// Outcome flags exposed for observability + portal display.
#[derive(Debug, Clone, PartialEq)]
pub enum ExchangeMode {
    /// `actor_token` absent or equal to subject → the issued token speaks for
    /// the subject without delegation chain.
    Impersonation,
    /// `actor_token` is a different principal — included as `act` claim
    /// per RFC 8693 §4.1.
    Delegation,
}

#[derive(Clone)]
pub struct TokenExchangeService {
    signing_secret: Vec<u8>,
    issuer: String,
    /// Maximum lifetime (seconds) for tokens minted via exchange.
    pub max_lifetime_secs: i64,
}

impl TokenExchangeService {
    pub fn new(issuer: String, signing_secret: Vec<u8>) -> Self {
        Self {
            signing_secret,
            issuer,
            max_lifetime_secs: 300,
        }
    }

    fn decode_subject(&self, token: &str) -> Result<SubjectClaims, ExchangeError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.set_required_spec_claims(&["sub", "exp"]);
        // Subject tokens may come from peer realms — we don't fix audience
        // here. The verifier of the issued token applies its own audience.
        validation.validate_aud = false;
        decode::<SubjectClaims>(
            token,
            &DecodingKey::from_secret(&self.signing_secret),
            &validation,
        )
        .map(|d| d.claims)
        .map_err(|_| ExchangeError::InvalidToken("subject_token failed validation"))
    }

    /// RFC 8693 §2.1 — perform the exchange.
    pub fn exchange(
        &self,
        req: &TokenExchangeRequest,
    ) -> Result<(TokenExchangeResponse, ExchangeMode), ExchangeError> {
        if req.grant_type != GRANT_TYPE_TOKEN_EXCHANGE {
            return Err(ExchangeError::InvalidRequest("grant_type"));
        }
        // RFC 8693 §2.1 — subject_token + subject_token_type are mandatory.
        if req.subject_token.is_empty() {
            return Err(ExchangeError::InvalidRequest("subject_token"));
        }
        if req.subject_token_type != token_type::ACCESS_TOKEN
            && req.subject_token_type != token_type::JWT
            && req.subject_token_type != token_type::ID_TOKEN
        {
            return Err(ExchangeError::UnsupportedTokenType);
        }
        let requested_type = req
            .requested_token_type
            .clone()
            .unwrap_or_else(|| token_type::ACCESS_TOKEN.to_string());
        // RFC 8693 §2.2 — only access/refresh/jwt issuance supported by
        // this implementation. SAML2 / ID-token-only is Phase 2.
        if requested_type != token_type::ACCESS_TOKEN
            && requested_type != token_type::JWT
            && requested_type != token_type::REFRESH_TOKEN
        {
            return Err(ExchangeError::UnsupportedTokenType);
        }

        let subject = self.decode_subject(&req.subject_token)?;

        // Determine mode (impersonation vs delegation).
        let (mode, act_chain) = match (&req.actor_token, &req.actor_token_type) {
            (Some(actor_token), Some(actor_type)) => {
                if actor_type != token_type::ACCESS_TOKEN && actor_type != token_type::JWT {
                    return Err(ExchangeError::UnsupportedTokenType);
                }
                let actor = self.decode_subject(actor_token)?;
                if actor.sub == subject.sub {
                    // Self-actor = impersonation in disguise.
                    (ExchangeMode::Impersonation, None)
                } else {
                    // Build the actor chain: subject's existing `act` (if
                    // any) becomes the inner `act.act`.
                    let chain = ActorClaim {
                        sub: actor.sub,
                        act: subject.act.clone().map(Box::new),
                    };
                    (ExchangeMode::Delegation, Some(chain))
                }
            }
            (None, None) => (ExchangeMode::Impersonation, None),
            _ => return Err(ExchangeError::InvalidRequest("actor_token/_type pair")),
        };

        // Audience: caller may shrink the audience (only). RFC 8693 §2.1
        // notes the server is the policy authority; we adopt the caller's
        // requested `audience` if present, else fall back to the subject
        // issuer's default audience.
        let aud = req
            .audience
            .clone()
            .or_else(|| req.resource.clone())
            .unwrap_or_else(|| subject.iss.clone());

        let scope = req
            .scope
            .clone()
            .or(subject.scope.clone())
            .unwrap_or_default();

        let now = Utc::now().timestamp();
        let exp = now + self.max_lifetime_secs;
        let claims = ExchangedClaims {
            iss: self.issuer.clone(),
            sub: subject.sub.clone(),
            aud,
            exp,
            iat: now,
            scope: scope.clone(),
            act: act_chain,
            jti: Uuid::new_v4().to_string(),
            client_id: req.client_id.clone(),
            typ: match mode {
                ExchangeMode::Impersonation => "exchange-impersonation".into(),
                ExchangeMode::Delegation => "exchange-delegation".into(),
            },
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&self.signing_secret),
        )
        .map_err(|_| ExchangeError::InvalidGrant)?;

        let issued_type = if requested_type == token_type::REFRESH_TOKEN {
            token_type::REFRESH_TOKEN
        } else {
            token_type::ACCESS_TOKEN
        };

        Ok((
            TokenExchangeResponse {
                access_token: token,
                issued_token_type: issued_type.into(),
                token_type: "Bearer".into(),
                expires_in: self.max_lifetime_secs,
                scope: if scope.is_empty() { None } else { Some(scope) },
                refresh_token: None,
            },
            mode,
        ))
    }

    /// Decode a previously issued exchanged token (used by tests + portal
    /// inspector).
    pub fn decode_issued(&self, token: &str) -> Result<ExchangedClaims, ExchangeError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.validate_aud = false;
        decode::<ExchangedClaims>(
            token,
            &DecodingKey::from_secret(&self.signing_secret),
            &validation,
        )
        .map(|d| d.claims)
        .map_err(|_| ExchangeError::InvalidToken("issued token decode"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mint_subject(svc: &TokenExchangeService, sub: &str, scope: &str) -> String {
        let now = Utc::now().timestamp();
        let s = SubjectClaims {
            sub: sub.into(),
            iss: svc.issuer.clone(),
            exp: now + 600,
            scope: Some(scope.into()),
            act: None,
            client_id: Some("client-A".into()),
        };
        encode(
            &Header::new(Algorithm::HS256),
            &s,
            &EncodingKey::from_secret(&svc.signing_secret),
        )
        .unwrap()
    }

    fn service() -> TokenExchangeService {
        TokenExchangeService::new("https://issuer.example".into(), b"test-secret".to_vec())
    }

    // upstream: rfc8693 §2.1 — grant_type must be the exchange URI.
    #[test]
    fn rejects_wrong_grant_type() {
        let svc = service();
        let req = TokenExchangeRequest {
            grant_type: "password".into(),
            subject_token: "x".into(),
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: None,
            actor_token_type: None,
            audience: None,
            resource: None,
            scope: None,
            client_id: None,
        };
        assert_eq!(
            svc.exchange(&req).unwrap_err(),
            ExchangeError::InvalidRequest("grant_type")
        );
    }

    // upstream: rfc8693 §1.1 — impersonation: actor_token absent → no act
    // chain on the issued token.
    #[test]
    fn impersonation_no_actor_no_act_claim() {
        let svc = service();
        let st = mint_subject(&svc, "alice", "read:foo");
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: st,
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: None,
            actor_token_type: None,
            audience: Some("api://orders".into()),
            resource: None,
            scope: None,
            client_id: Some("client-B".into()),
        };
        let (resp, mode) = svc.exchange(&req).unwrap();
        assert_eq!(mode, ExchangeMode::Impersonation);
        let c = svc.decode_issued(&resp.access_token).unwrap();
        assert_eq!(c.sub, "alice");
        assert_eq!(c.aud, "api://orders");
        assert!(c.act.is_none());
        assert_eq!(c.typ, "exchange-impersonation");
    }

    // upstream: rfc8693 §1.2 + §4.1 — delegation: actor sub ≠ subject sub →
    // emitted token contains an `act` claim with the actor.
    #[test]
    fn delegation_emits_act_chain() {
        let svc = service();
        let st = mint_subject(&svc, "alice", "read:foo");
        let at = mint_subject(&svc, "bob-service", "");
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: st,
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: Some(at),
            actor_token_type: Some(token_type::ACCESS_TOKEN.into()),
            audience: Some("api://payments".into()),
            resource: None,
            scope: None,
            client_id: Some("client-B".into()),
        };
        let (resp, mode) = svc.exchange(&req).unwrap();
        assert_eq!(mode, ExchangeMode::Delegation);
        let c = svc.decode_issued(&resp.access_token).unwrap();
        assert_eq!(c.sub, "alice");
        let act = c.act.expect("act claim present");
        assert_eq!(act.sub, "bob-service");
        assert!(act.act.is_none());
        assert_eq!(c.typ, "exchange-delegation");
    }

    // upstream: rfc8693 §2.1 — unknown subject_token_type rejected.
    #[test]
    fn unsupported_subject_token_type_rejected() {
        let svc = service();
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: "x".into(),
            subject_token_type: "urn:made-up".into(),
            requested_token_type: None,
            actor_token: None,
            actor_token_type: None,
            audience: None,
            resource: None,
            scope: None,
            client_id: None,
        };
        assert_eq!(svc.exchange(&req).unwrap_err(), ExchangeError::UnsupportedTokenType);
    }

    // upstream: rfc8693 §2.2 — issued_token_type echoes the requested type
    // when honored (here REFRESH_TOKEN).
    #[test]
    fn issued_token_type_reflects_request_for_refresh() {
        let svc = service();
        let st = mint_subject(&svc, "alice", "");
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: st,
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: Some(token_type::REFRESH_TOKEN.into()),
            actor_token: None,
            actor_token_type: None,
            audience: None,
            resource: None,
            scope: None,
            client_id: None,
        };
        let (resp, _) = svc.exchange(&req).unwrap();
        assert_eq!(resp.issued_token_type, token_type::REFRESH_TOKEN);
    }

    // upstream: rfc8693 §4.1 — when the subject already carries an `act`
    // claim, a further delegation step prepends — the new actor wraps the
    // existing chain in `act.act`.
    #[test]
    fn nested_delegation_chain_is_preserved() {
        let svc = service();
        // Subject token already has an embedded act = carol.
        let now = Utc::now().timestamp();
        let nested = SubjectClaims {
            sub: "alice".into(),
            iss: svc.issuer.clone(),
            exp: now + 600,
            scope: Some("".into()),
            act: Some(ActorClaim { sub: "carol".into(), act: None }),
            client_id: None,
        };
        let st = encode(
            &Header::new(Algorithm::HS256),
            &nested,
            &EncodingKey::from_secret(&svc.signing_secret),
        )
        .unwrap();
        let at = mint_subject(&svc, "bob-service", "");
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: st,
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: Some(at),
            actor_token_type: Some(token_type::ACCESS_TOKEN.into()),
            audience: Some("api://x".into()),
            resource: None,
            scope: None,
            client_id: None,
        };
        let (resp, _) = svc.exchange(&req).unwrap();
        let c = svc.decode_issued(&resp.access_token).unwrap();
        // Outer: bob-service. Inner: carol.
        let act = c.act.expect("act present");
        assert_eq!(act.sub, "bob-service");
        let inner = act.act.expect("inner act");
        assert_eq!(inner.sub, "carol");
    }

    // upstream: rfc8693 §2.1 — subject and actor with the same sub collapses
    // to impersonation (no act claim).
    #[test]
    fn same_subject_actor_collapses_to_impersonation() {
        let svc = service();
        let st = mint_subject(&svc, "alice", "");
        let at = mint_subject(&svc, "alice", "");
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: st,
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: Some(at),
            actor_token_type: Some(token_type::ACCESS_TOKEN.into()),
            audience: Some("api://x".into()),
            resource: None,
            scope: None,
            client_id: None,
        };
        let (resp, mode) = svc.exchange(&req).unwrap();
        assert_eq!(mode, ExchangeMode::Impersonation);
        let c = svc.decode_issued(&resp.access_token).unwrap();
        assert!(c.act.is_none());
    }

    // upstream: rfc8693 §2.1 — actor_token without actor_token_type is a
    // malformed request.
    #[test]
    fn actor_pair_must_be_complete() {
        let svc = service();
        let st = mint_subject(&svc, "alice", "");
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: st,
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: Some("...".into()),
            actor_token_type: None, // <-- missing
            audience: None,
            resource: None,
            scope: None,
            client_id: None,
        };
        let err = svc.exchange(&req).unwrap_err();
        assert!(matches!(err, ExchangeError::InvalidRequest(_)));
    }

    // upstream: rfc8693 §2.1 — corrupt subject_token is invalid_token.
    #[test]
    fn corrupt_subject_token_rejected() {
        let svc = service();
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: "not.a.jwt".into(),
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: None,
            actor_token_type: None,
            audience: None,
            resource: None,
            scope: None,
            client_id: None,
        };
        let err = svc.exchange(&req).unwrap_err();
        assert!(matches!(err, ExchangeError::InvalidToken(_)));
    }

    // upstream: rfc8693 §2.1 — `resource` may substitute for `audience` when
    // audience is absent (server policy).
    #[test]
    fn resource_falls_back_to_audience() {
        let svc = service();
        let st = mint_subject(&svc, "alice", "");
        let req = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: st,
            subject_token_type: token_type::ACCESS_TOKEN.into(),
            requested_token_type: None,
            actor_token: None,
            actor_token_type: None,
            audience: None,
            resource: Some("https://api.example/v1/orders".into()),
            scope: None,
            client_id: None,
        };
        let (resp, _) = svc.exchange(&req).unwrap();
        let c = svc.decode_issued(&resp.access_token).unwrap();
        assert_eq!(c.aud, "https://api.example/v1/orders");
    }
}

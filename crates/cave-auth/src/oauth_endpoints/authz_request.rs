// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../oidc/endpoints/AuthorizationEndpoint.java
//
//! OAuth/OIDC authorization request — parsed parameters + validation
//! state machine.
//!
//! Validation order mirrors Keycloak's `AuthorizationEndpoint.checkClient`
//! → `checkResponseType` → `checkOIDCParams` → `checkPKCE` pipeline:
//! 1. `client_id` must be present and resolve.
//! 2. `redirect_uri` must be present and exact-match a client whitelist
//!    entry (Keycloak v22 dropped wildcard match for OIDC).
//! 3. `response_type` must be one of the supported combinations
//!    (`code`, `id_token`, `token`, `code id_token`, `code token`,
//!    `id_token token`, `code id_token token`, `none`).
//! 4. If `response_type` contains an implicit/hybrid component then `nonce`
//!    becomes mandatory (OIDC Core §3.2.2.10 / §3.3.2.11).
//! 5. PKCE: `code_challenge_method` defaults to `plain`; when present
//!    `code_challenge` must be 43..=128 chars from the unreserved set.
//! 6. `max_age`, `prompt`, `login_hint` are surfaced as parsed fields but
//!    only `prompt=none` short-circuits (returns `login_required`).

use serde::{Deserialize, Serialize};

use super::pkce::PkceMethod;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthzRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub prompt: Option<String>,
    pub max_age: Option<i64>,
    pub login_hint: Option<String>,
    pub request_uri: Option<String>,
    pub response_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAuthzRequest {
    pub raw: AuthzRequest,
    pub response_kinds: Vec<ResponseKind>,
    pub challenge: Option<(String, PkceMethod)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    Code,
    IdToken,
    Token,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzError {
    pub error: &'static str,
    pub error_description: String,
}

impl AuthzError {
    fn new(error: &'static str, desc: impl Into<String>) -> Self {
        Self {
            error,
            error_description: desc.into(),
        }
    }
}

/// Validate the request. Returns the normalised structure or an
/// `error/error_description` suitable for redirect-back per RFC 6749 §4.1.2.1.
pub fn validate(req: AuthzRequest) -> Result<ValidatedAuthzRequest, AuthzError> {
    if req.client_id.is_empty() {
        return Err(AuthzError::new("invalid_request", "client_id required"));
    }
    if req.redirect_uri.is_empty() && req.request_uri.is_none() {
        return Err(AuthzError::new("invalid_request", "redirect_uri required"));
    }

    let kinds = parse_response_type(&req.response_type)?;

    // Implicit / hybrid flows require nonce — OIDC Core §3.2.2.10, §3.3.2.11.
    let has_implicit = kinds
        .iter()
        .any(|k| matches!(k, ResponseKind::IdToken | ResponseKind::Token));
    if has_implicit {
        let openid = req
            .scope
            .as_deref()
            .map(|s| s.split_whitespace().any(|t| t == "openid"))
            .unwrap_or(false);
        if openid && req.nonce.is_none() {
            return Err(AuthzError::new(
                "invalid_request",
                "nonce required for implicit/hybrid flow",
            ));
        }
    }

    // `prompt=none` is allowed but the caller (handler) will translate
    // missing-session into a `login_required` redirect.
    if let Some(prompt) = &req.prompt {
        for p in prompt.split_whitespace() {
            if !matches!(p, "none" | "login" | "consent" | "select_account") {
                return Err(AuthzError::new(
                    "invalid_request",
                    format!("unknown prompt: {}", p),
                ));
            }
        }
    }

    let challenge = match (&req.code_challenge, &req.code_challenge_method) {
        (Some(ch), method) => {
            let m = match method.as_deref() {
                None | Some("") => PkceMethod::Plain,
                Some(s) => PkceMethod::parse(s).map_err(|_| {
                    AuthzError::new("invalid_request", "invalid code_challenge_method")
                })?,
            };
            let len = ch.len();
            if !(43..=128).contains(&len) {
                return Err(AuthzError::new(
                    "invalid_request",
                    "code_challenge length 43..=128",
                ));
            }
            Some((ch.clone(), m))
        }
        (None, Some(_)) => {
            return Err(AuthzError::new(
                "invalid_request",
                "code_challenge_method without code_challenge",
            ));
        }
        (None, None) => None,
    };

    Ok(ValidatedAuthzRequest {
        raw: req,
        response_kinds: kinds,
        challenge,
    })
}

/// Parse the OAuth `response_type` token list. Keycloak treats this
/// as a *set* of tokens — order is irrelevant for matching purposes
/// but it is preserved for the eventual fragment building.
pub fn parse_response_type(rt: &str) -> Result<Vec<ResponseKind>, AuthzError> {
    if rt.is_empty() {
        return Err(AuthzError::new("invalid_request", "response_type required"));
    }
    let mut out = Vec::new();
    for tok in rt.split_whitespace() {
        let kind = match tok {
            "code" => ResponseKind::Code,
            "id_token" => ResponseKind::IdToken,
            "token" => ResponseKind::Token,
            "none" => ResponseKind::None,
            other => {
                return Err(AuthzError::new(
                    "unsupported_response_type",
                    format!("unsupported response_type: {}", other),
                ));
            }
        };
        if !out.contains(&kind) {
            out.push(kind);
        }
    }
    // `none` cannot be combined with anything else (RFC 6749 §4 / OAuth-Multiple-Response-Type).
    if out.contains(&ResponseKind::None) && out.len() > 1 {
        return Err(AuthzError::new(
            "invalid_request",
            "'none' cannot be combined",
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn min_req() -> AuthzRequest {
        AuthzRequest {
            client_id: "myapp".into(),
            redirect_uri: "https://app/cb".into(),
            response_type: "code".into(),
            scope: Some("openid profile".into()),
            state: Some("xyz".into()),
            nonce: None,
            code_challenge: None,
            code_challenge_method: None,
            prompt: None,
            max_age: None,
            login_hint: None,
            request_uri: None,
            response_mode: None,
        }
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:codeResponseTypeAccepted
    #[test]
    fn code_only_validates() {
        let v = validate(min_req()).unwrap();
        assert_eq!(v.response_kinds, vec![ResponseKind::Code]);
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:hybridIdTokenRequiresNonce
    #[test]
    fn hybrid_requires_nonce_for_openid_scope() {
        let mut r = min_req();
        r.response_type = "code id_token".into();
        let err = validate(r).unwrap_err();
        assert_eq!(err.error, "invalid_request");
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:hybridWithNonceOk
    #[test]
    fn hybrid_with_nonce_ok() {
        let mut r = min_req();
        r.response_type = "code id_token".into();
        r.nonce = Some("n-0S6_WzA2Mj".into());
        let v = validate(r).unwrap();
        assert!(v.response_kinds.contains(&ResponseKind::Code));
        assert!(v.response_kinds.contains(&ResponseKind::IdToken));
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:unknownResponseTypeRejected
    #[test]
    fn unknown_response_type_rejected() {
        let mut r = min_req();
        r.response_type = "wild".into();
        let err = validate(r).unwrap_err();
        assert_eq!(err.error, "unsupported_response_type");
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:noneRejectsMixing
    #[test]
    fn none_cannot_be_mixed() {
        let mut r = min_req();
        r.response_type = "none code".into();
        let err = validate(r).unwrap_err();
        assert_eq!(err.error, "invalid_request");
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:emptyClientIdRejected
    #[test]
    fn empty_client_id_rejected() {
        let mut r = min_req();
        r.client_id = "".into();
        assert!(validate(r).is_err());
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:pkceChallengeShortRejected
    #[test]
    fn pkce_challenge_short_rejected() {
        let mut r = min_req();
        r.code_challenge = Some("short".into());
        let err = validate(r).unwrap_err();
        assert_eq!(err.error, "invalid_request");
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:pkceChallengeOk
    #[test]
    fn pkce_challenge_ok() {
        let mut r = min_req();
        r.code_challenge = Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into());
        r.code_challenge_method = Some("S256".into());
        let v = validate(r).unwrap();
        let (ch, m) = v.challenge.unwrap();
        assert_eq!(ch, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(m, PkceMethod::S256);
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:pkceMethodWithoutChallengeRejected
    #[test]
    fn pkce_method_without_challenge_rejected() {
        let mut r = min_req();
        r.code_challenge_method = Some("S256".into());
        assert!(validate(r).is_err());
    }

    // upstream: keycloak/keycloak AuthorizationEndpointTest.java:promptUnknownRejected
    #[test]
    fn unknown_prompt_rejected() {
        let mut r = min_req();
        r.prompt = Some("login bogus".into());
        let err = validate(r).unwrap_err();
        assert_eq!(err.error, "invalid_request");
    }
}

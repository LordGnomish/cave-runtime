// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/TokenExchangeGrantType.java + RFC 8693 §2.1, §4.1
//
//! `actor_token` (delegation) handling.
//!
//! When the requesting client wants to perform an action on behalf of `subject`
//! but flag itself as the actor (`act` claim in the issued JWT, RFC 8693 §4.1),
//! it supplies both `subject_token` and `actor_token`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::subject_token::{SubjectToken, SubjectTokenError, SubjectTokenType};

/// The `act` claim per RFC 8693 §4.1.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActorClaim {
    pub sub: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub iss: Option<String>,
    /// If the actor was itself acting on behalf of someone, RFC 8693 says you
    /// MAY nest another `act` claim (§4.1 example with `act -> act`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub act: Option<Box<ActorClaim>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActorTokenError {
    #[error("actor_token_type is required when actor_token is supplied")]
    TypeRequired,
    #[error("actor_token parse failed: {0}")]
    Parse(#[from] SubjectTokenError),
}

/// Parse the actor side of a delegation exchange.
pub fn parse_actor(
    actor_token: &str,
    actor_token_type: &str,
) -> Result<ActorClaim, ActorTokenError> {
    if actor_token_type.is_empty() {
        return Err(ActorTokenError::TypeRequired);
    }
    let t: SubjectTokenType = actor_token_type
        .parse()
        .map_err(ActorTokenError::Parse)?;
    let parsed = SubjectToken::parse(actor_token, t).map_err(ActorTokenError::Parse)?;
    Ok(ActorClaim {
        sub: parsed.subject,
        iss: parsed.issuer,
        act: None,
    })
}

/// Folds an actor claim onto an existing one to support nested delegation
/// (RFC 8693 §4.1 example 2).
pub fn nest_actor(outer: ActorClaim, inner: Option<ActorClaim>) -> ActorClaim {
    let mut o = outer;
    if let Some(i) = inner {
        o.act = Some(Box::new(i));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn b64u(s: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.as_bytes())
    }

    fn jwt(body: &str) -> String {
        format!("{}.{}.{}", b64u(r#"{"alg":"none"}"#), b64u(body), b64u("sig"))
    }

    #[test]
    fn empty_type_rejected() {
        assert_eq!(
            parse_actor("anything", ""),
            Err(ActorTokenError::TypeRequired)
        );
    }

    #[test]
    fn unknown_type_rejected() {
        let err = parse_actor("anything", "bad").unwrap_err();
        assert!(matches!(err, ActorTokenError::Parse(_)));
    }

    #[test]
    fn parses_jwt_actor() {
        let t = jwt(r#"{"sub":"service-x","iss":"https://idp"}"#);
        let a = parse_actor(&t, SubjectTokenType::AccessToken.as_uri()).unwrap();
        assert_eq!(a.sub, "service-x");
        assert_eq!(a.iss.as_deref(), Some("https://idp"));
        assert!(a.act.is_none());
    }

    #[test]
    fn nesting_adds_inner_actor() {
        let outer = ActorClaim {
            sub: "frontend".into(),
            iss: None,
            act: None,
        };
        let inner = ActorClaim {
            sub: "browser".into(),
            iss: None,
            act: None,
        };
        let nested = nest_actor(outer, Some(inner));
        assert_eq!(nested.act.unwrap().sub, "browser");
    }

    #[test]
    fn nesting_no_inner_is_noop() {
        let outer = ActorClaim {
            sub: "frontend".into(),
            iss: None,
            act: None,
        };
        let result = nest_actor(outer.clone(), None);
        assert!(result.act.is_none());
        assert_eq!(result, outer);
    }

    #[test]
    fn deep_nesting_serialises() {
        let inner = ActorClaim {
            sub: "browser".into(),
            iss: None,
            act: None,
        };
        let middle = ActorClaim {
            sub: "gateway".into(),
            iss: None,
            act: Some(Box::new(inner)),
        };
        let outer = ActorClaim {
            sub: "rs".into(),
            iss: None,
            act: Some(Box::new(middle)),
        };
        let json = serde_json::to_string(&outer).unwrap();
        // Ensure all three levels survive the round-trip.
        assert!(json.contains("rs"));
        assert!(json.contains("gateway"));
        assert!(json.contains("browser"));
    }
}

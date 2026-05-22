// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/TokenExchangeGrantType.java + RFC 8693 §2.1
//
//! `requested_token_type` + `audience` parameter handling.
//!
//! RFC 8693 §2.1 lets the client request a token whose `aud` differs from
//! the subject_token's `aud`. The AS must validate the request against an
//! exchange policy and mint accordingly.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::subject_token::SubjectTokenType;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudienceRequest {
    /// One or more `audience` parameter values (RFC 8693 §2.1 allows
    /// repetition).
    pub audiences: Vec<String>,
    /// Requested token type to mint.
    pub requested_type: SubjectTokenType,
    /// Optional `resource` parameter (URI of the protected resource).
    pub resources: Vec<String>,
    /// Optional scope.
    pub scopes: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AudienceError {
    #[error("at least one audience or resource must be supplied")]
    NoTarget,
    #[error("audience {0:?} is not allowed for client {1:?}")]
    Forbidden(String, String),
    #[error("requested_token_type {0:?} is unknown")]
    UnknownRequestedType(String),
}

impl AudienceRequest {
    pub fn new(
        audiences: Vec<String>,
        resources: Vec<String>,
        requested_type: SubjectTokenType,
        scopes: Vec<String>,
    ) -> Result<Self, AudienceError> {
        if audiences.is_empty() && resources.is_empty() {
            return Err(AudienceError::NoTarget);
        }
        Ok(Self {
            audiences,
            resources,
            requested_type,
            scopes,
        })
    }

    /// Effective target — RFC 8693 §3 says the response's `aud` claim MUST be
    /// the requested audience(s). We pick a single canonical aud if exactly
    /// one is requested, otherwise return the full list as JSON-array string.
    pub fn primary_aud(&self) -> Option<&str> {
        if self.audiences.len() == 1 {
            Some(&self.audiences[0])
        } else {
            self.audiences.first().map(|s| s.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_least_one_target_required() {
        let err = AudienceRequest::new(vec![], vec![], SubjectTokenType::AccessToken, vec![])
            .unwrap_err();
        assert_eq!(err, AudienceError::NoTarget);
    }

    #[test]
    fn single_audience_primary() {
        let r = AudienceRequest::new(
            vec!["billing".into()],
            vec![],
            SubjectTokenType::AccessToken,
            vec![],
        )
        .unwrap();
        assert_eq!(r.primary_aud(), Some("billing"));
    }

    #[test]
    fn multi_audience_first_is_primary() {
        let r = AudienceRequest::new(
            vec!["a".into(), "b".into()],
            vec![],
            SubjectTokenType::Jwt,
            vec![],
        )
        .unwrap();
        assert_eq!(r.primary_aud(), Some("a"));
    }

    #[test]
    fn resource_only_request_allowed() {
        let r = AudienceRequest::new(
            vec![],
            vec!["https://api.cave.dev".into()],
            SubjectTokenType::AccessToken,
            vec![],
        )
        .unwrap();
        assert!(r.audiences.is_empty());
        assert_eq!(r.resources.len(), 1);
    }

    #[test]
    fn scopes_preserved() {
        let r = AudienceRequest::new(
            vec!["aud".into()],
            vec![],
            SubjectTokenType::Jwt,
            vec!["read".into(), "write".into()],
        )
        .unwrap();
        assert_eq!(r.scopes, vec!["read", "write"]);
    }

    #[test]
    fn requested_type_round_trips() {
        let r = AudienceRequest::new(vec!["a".into()], vec![], SubjectTokenType::IdToken, vec![])
            .unwrap();
        assert_eq!(r.requested_type, SubjectTokenType::IdToken);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/utils/AccessTokenExchanger.java + RFC 8693 §2.1
//
//! `subject_token` + `subject_token_type` validation.
//!
//! The subject token is the credential presented by the client representing
//! the principal whose authority is being exchanged.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// RFC 8693 §3 — registered token type URIs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubjectTokenType {
    AccessToken,
    RefreshToken,
    IdToken,
    Saml1,
    Saml2,
    Jwt,
}

impl SubjectTokenType {
    pub const ACCESS_TOKEN: &'static str =
        "urn:ietf:params:oauth:token-type:access_token";
    pub const REFRESH_TOKEN: &'static str =
        "urn:ietf:params:oauth:token-type:refresh_token";
    pub const ID_TOKEN: &'static str = "urn:ietf:params:oauth:token-type:id_token";
    pub const SAML1: &'static str = "urn:ietf:params:oauth:token-type:saml1";
    pub const SAML2: &'static str = "urn:ietf:params:oauth:token-type:saml2";
    pub const JWT: &'static str = "urn:ietf:params:oauth:token-type:jwt";

    pub fn as_uri(&self) -> &'static str {
        match self {
            SubjectTokenType::AccessToken => Self::ACCESS_TOKEN,
            SubjectTokenType::RefreshToken => Self::REFRESH_TOKEN,
            SubjectTokenType::IdToken => Self::ID_TOKEN,
            SubjectTokenType::Saml1 => Self::SAML1,
            SubjectTokenType::Saml2 => Self::SAML2,
            SubjectTokenType::Jwt => Self::JWT,
        }
    }
}

impl FromStr for SubjectTokenType {
    type Err = SubjectTokenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            Self::ACCESS_TOKEN => SubjectTokenType::AccessToken,
            Self::REFRESH_TOKEN => SubjectTokenType::RefreshToken,
            Self::ID_TOKEN => SubjectTokenType::IdToken,
            Self::SAML1 => SubjectTokenType::Saml1,
            Self::SAML2 => SubjectTokenType::Saml2,
            Self::JWT => SubjectTokenType::Jwt,
            other => return Err(SubjectTokenError::UnknownType(other.to_string())),
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SubjectTokenError {
    #[error("unknown subject_token_type {0:?}")]
    UnknownType(String),
    #[error("subject_token must be a 3-segment JWT for type {0:?}")]
    NotJwt(&'static str),
    #[error("subject_token JWT body invalid: {0}")]
    JwtParse(String),
    #[error("SAML token must be base64 of `<saml:Assertion>` or `<samlp:Response>`: {0}")]
    Saml(String),
}

/// Decoded subject token + the principal claim.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SubjectToken {
    pub token_type: SubjectTokenType,
    /// The original raw bearer string — kept so the downstream can re-introspect.
    pub raw: String,
    /// Subject claim — `sub` for JWT, NameID for SAML2.
    pub subject: String,
    /// Optional issuer.
    pub issuer: Option<String>,
    /// Optional audience.
    pub audience: Option<String>,
}

impl SubjectToken {
    /// Parses a `subject_token` of the declared type.
    pub fn parse(token: &str, token_type: SubjectTokenType) -> Result<Self, SubjectTokenError> {
        match token_type {
            SubjectTokenType::AccessToken
            | SubjectTokenType::IdToken
            | SubjectTokenType::Jwt
            | SubjectTokenType::RefreshToken => parse_jwt(token, token_type),
            SubjectTokenType::Saml1 | SubjectTokenType::Saml2 => parse_saml(token, token_type),
        }
    }
}

fn parse_jwt(token: &str, t: SubjectTokenType) -> Result<SubjectToken, SubjectTokenError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(SubjectTokenError::NotJwt(t.as_uri()));
    }
    let body = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1].as_bytes())
        .map_err(|e| SubjectTokenError::JwtParse(e.to_string()))?;
    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| SubjectTokenError::JwtParse(e.to_string()))?;

    let subject = v
        .get("sub")
        .and_then(|x| x.as_str())
        .ok_or_else(|| SubjectTokenError::JwtParse("missing sub".into()))?
        .to_string();
    let issuer = v.get("iss").and_then(|x| x.as_str()).map(str::to_string);
    let audience = v.get("aud").and_then(|x| x.as_str()).map(str::to_string);

    Ok(SubjectToken {
        token_type: t,
        raw: token.to_string(),
        subject,
        issuer,
        audience,
    })
}

fn parse_saml(token: &str, t: SubjectTokenType) -> Result<SubjectToken, SubjectTokenError> {
    // SAML subject tokens are base64(XML). We don't run the full SAML parser
    // here (saml/ module owns that); for token-exchange purposes we need the
    // NameID — extract it with a minimal substring scan to keep coupling low.
    let xml = base64::engine::general_purpose::STANDARD
        .decode(token.as_bytes())
        .map_err(|e| SubjectTokenError::Saml(format!("bad base64: {e}")))?;
    let xml = String::from_utf8(xml).map_err(|e| SubjectTokenError::Saml(e.to_string()))?;
    let nameid_start = xml
        .find("<saml:NameID")
        .or_else(|| xml.find("<NameID"))
        .ok_or_else(|| SubjectTokenError::Saml("no NameID element".into()))?;
    let body_start = xml[nameid_start..]
        .find('>')
        .ok_or_else(|| SubjectTokenError::Saml("malformed NameID open tag".into()))?
        + nameid_start
        + 1;
    let body_end = xml[body_start..]
        .find('<')
        .ok_or_else(|| SubjectTokenError::Saml("unterminated NameID".into()))?
        + body_start;
    let subject = xml[body_start..body_end].to_string();
    Ok(SubjectToken {
        token_type: t,
        raw: token.to_string(),
        subject,
        issuer: None,
        audience: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64u(s: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.as_bytes())
    }

    fn jwt(body: &str) -> String {
        format!("{}.{}.{}", b64u(r#"{"alg":"none"}"#), b64u(body), b64u("sig"))
    }

    #[test]
    fn type_uri_roundtrip() {
        for t in [
            SubjectTokenType::AccessToken,
            SubjectTokenType::RefreshToken,
            SubjectTokenType::IdToken,
            SubjectTokenType::Jwt,
            SubjectTokenType::Saml1,
            SubjectTokenType::Saml2,
        ] {
            assert_eq!(SubjectTokenType::from_str(t.as_uri()).unwrap(), t);
        }
    }

    #[test]
    fn unknown_type_rejected() {
        assert!(matches!(
            SubjectTokenType::from_str("nope").unwrap_err(),
            SubjectTokenError::UnknownType(_)
        ));
    }

    #[test]
    fn jwt_subject_extracted() {
        let t = jwt(r#"{"sub":"alice","iss":"https://idp","aud":"rs"}"#);
        let parsed = SubjectToken::parse(&t, SubjectTokenType::AccessToken).unwrap();
        assert_eq!(parsed.subject, "alice");
        assert_eq!(parsed.issuer.as_deref(), Some("https://idp"));
        assert_eq!(parsed.audience.as_deref(), Some("rs"));
    }

    #[test]
    fn jwt_without_sub_rejected() {
        let t = jwt(r#"{"foo":"bar"}"#);
        let err = SubjectToken::parse(&t, SubjectTokenType::Jwt).unwrap_err();
        assert!(matches!(err, SubjectTokenError::JwtParse(_)));
    }

    #[test]
    fn jwt_not_three_segments_rejected() {
        let err = SubjectToken::parse("only.two", SubjectTokenType::Jwt).unwrap_err();
        assert!(matches!(err, SubjectTokenError::NotJwt(_)));
    }

    #[test]
    fn saml2_nameid_extracted() {
        let xml = r#"<saml:Response xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"><saml:Assertion><saml:Subject><saml:NameID>bob</saml:NameID></saml:Subject></saml:Assertion></saml:Response>"#;
        let token = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let parsed = SubjectToken::parse(&token, SubjectTokenType::Saml2).unwrap();
        assert_eq!(parsed.subject, "bob");
    }

    #[test]
    fn saml_without_nameid_rejected() {
        let xml = r#"<saml:Response></saml:Response>"#;
        let token = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let err = SubjectToken::parse(&token, SubjectTokenType::Saml2).unwrap_err();
        assert!(matches!(err, SubjectTokenError::Saml(_)));
    }

    #[test]
    fn id_token_type_recognised() {
        let t = jwt(r#"{"sub":"u"}"#);
        let parsed = SubjectToken::parse(&t, SubjectTokenType::IdToken).unwrap();
        assert_eq!(parsed.token_type, SubjectTokenType::IdToken);
    }

    #[test]
    fn refresh_token_type_recognised() {
        let t = jwt(r#"{"sub":"u"}"#);
        let parsed = SubjectToken::parse(&t, SubjectTokenType::RefreshToken).unwrap();
        assert_eq!(parsed.token_type, SubjectTokenType::RefreshToken);
    }

    #[test]
    fn unbarred_jwt_subject_keeps_raw() {
        let raw = jwt(r#"{"sub":"alice"}"#);
        let parsed = SubjectToken::parse(&raw, SubjectTokenType::AccessToken).unwrap();
        assert_eq!(parsed.raw, raw);
    }
}

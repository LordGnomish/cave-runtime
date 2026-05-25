// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/wsfed/

//! WS-Federation 1.x protocol — port of Keycloak's `protocol/wsfed/`.
//!
//! WS-Federation (WS-Fed) is the federation protocol Microsoft platforms
//! (AD FS, Azure AD legacy, SharePoint, Dynamics) speak before they pick up
//! OIDC. It wraps a **SAML 1.1** assertion (note: 1.1, not 2.0) in a
//! WS-Trust `RequestSecurityTokenResponse` (RSTR) envelope. The wire format
//! is:
//!
//! ```text
//! GET  /protocol/wsfed/{realm}?wa=wsignin1.0&wtrealm=...&wctx=...&wreply=...
//! POST /protocol/wsfed/{realm}    (wresult=<base64-encoded RSTR>)
//! GET  /protocol/wsfed/{realm}?wa=wsignout1.0&wreply=...
//! ```
//!
//! The four `wa` query parameters cave-auth recognises are:
//! * `wsignin1.0`  — SP-initiated sign-in (returns RSTR)
//! * `wsignout1.0` — SP-initiated sign-out
//! * `wsignoutcleanup1.0` — IdP-initiated cleanup
//! * `wattr1.0`    — attribute query (rare, scope-cut)
//!
//! ## What this module covers
//!
//! * [`protocol`]          — RST/RSTR XML codecs.
//! * [`saml11_assertion`]  — SAML **1.1** Assertion (NameIdentifier,
//!   AuthenticationStatement, AttributeStatement). Note the 1.1 namespace
//!   `urn:oasis:names:tc:SAML:1.0:assertion` — yes, 1.0; that's what
//!   SAML 1.1 actually uses on the wire.
//! * [`endpoints`]         — Axum router for `/protocol/wsfed/{realm}`.
//! * [`signing`]           — reuses RSA-SHA256 from [`crate::saml::signature`].
//!
//! ## Honest limitations
//!
//! * `wattr1.0` attribute query is not implemented — never seen in cave
//!   customer deployments.
//! * Token encryption (WS-Federation Active Profile) is out of scope —
//!   passive-profile only.
//! * RST callers usually send a `wctx` opaque blob and expect it echoed
//!   back unchanged; we preserve it verbatim.

pub mod endpoints;
pub mod protocol;
pub mod saml11_assertion;
pub mod signing;

use thiserror::Error;

/// Errors emitted from the WS-Fed surface.
#[derive(Debug, Error)]
pub enum WsFedError {
    /// XML parse failed (malformed input).
    #[error("WS-Fed XML parse error: {0}")]
    Parse(String),
    /// Required RST/RSTR field missing.
    #[error("WS-Fed missing field: {0}")]
    MissingField(String),
    /// Unknown / unsupported `wa` parameter.
    #[error("WS-Fed unsupported wa: {0}")]
    UnsupportedAction(String),
    /// SAML 1.1 assertion signing failed.
    #[error("WS-Fed signature error: {0}")]
    Signature(String),
}

/// `wa` action parameter — the WS-Fed verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsAction {
    /// `wsignin1.0` — sign-in request / response.
    Signin,
    /// `wsignout1.0` — sign-out request initiated by the relying party.
    Signout,
    /// `wsignoutcleanup1.0` — sign-out cleanup signal from the IdP.
    SignoutCleanup,
}

impl WsAction {
    pub fn as_str(self) -> &'static str {
        match self {
            WsAction::Signin => "wsignin1.0",
            WsAction::Signout => "wsignout1.0",
            WsAction::SignoutCleanup => "wsignoutcleanup1.0",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, WsFedError> {
        match s {
            "wsignin1.0" => Ok(WsAction::Signin),
            "wsignout1.0" => Ok(WsAction::Signout),
            "wsignoutcleanup1.0" => Ok(WsAction::SignoutCleanup),
            other => Err(WsFedError::UnsupportedAction(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_action_roundtrips() {
        for a in [
            WsAction::Signin,
            WsAction::Signout,
            WsAction::SignoutCleanup,
        ] {
            assert_eq!(WsAction::from_str(a.as_str()).unwrap(), a);
        }
    }

    #[test]
    fn ws_action_unknown_rejected() {
        assert!(WsAction::from_str("wattr1.0").is_err());
        assert!(WsAction::from_str("wsignin2.0").is_err());
        assert!(WsAction::from_str("").is_err());
    }
}

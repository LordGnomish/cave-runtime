// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/wsfed/

//! WS-Federation 1.x protocol — RED phase: tests are defined,
//! implementation lands in the GREEN commit.

pub mod endpoints;
pub mod protocol;
pub mod saml11_assertion;
pub mod signing;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WsFedError {
    #[error("WS-Fed XML parse error: {0}")]
    Parse(String),
    #[error("WS-Fed missing field: {0}")]
    MissingField(String),
    #[error("WS-Fed unsupported wa: {0}")]
    UnsupportedAction(String),
    #[error("WS-Fed signature error: {0}")]
    Signature(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsAction {
    Signin,
    Signout,
    SignoutCleanup,
}

impl WsAction {
    pub fn as_str(self) -> &'static str {
        // RED: deliberately wrong — GREEN will return the spec strings.
        ""
    }
    pub fn from_str(_s: &str) -> Result<Self, WsFedError> {
        Err(WsFedError::UnsupportedAction("not yet implemented".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_action_roundtrips() {
        for a in [WsAction::Signin, WsAction::Signout, WsAction::SignoutCleanup] {
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

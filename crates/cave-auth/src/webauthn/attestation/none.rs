// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// "none" attestation — W3C §8.7.
//
// Format: attStmt MUST be an empty CBOR map. Verification trivially succeeds;
// the caller is responsible for deciding whether to accept self-attestation /
// none against their RP policy.

use ciborium::value::Value;

use crate::webauthn::WebAuthnError;

pub fn parse(stmt: &Value) -> Result<(), WebAuthnError> {
    match stmt {
        Value::Map(m) if m.is_empty() => Ok(()),
        Value::Map(_) => Err(WebAuthnError::Attestation(
            "none attStmt must be empty map".into(),
        )),
        _ => Err(WebAuthnError::Attestation(
            "none attStmt not a map".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_empty_map() {
        parse(&Value::Map(vec![])).unwrap();
    }

    #[test]
    fn rejects_non_empty_map() {
        let v = Value::Map(vec![(
            Value::Text("alg".into()),
            Value::Integer(0i64.into()),
        )]);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn rejects_non_map() {
        assert!(parse(&Value::Text("hi".into())).is_err());
    }
}

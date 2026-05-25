// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// Attestation-statement registry — webauthn4j `verifier.attestation.statement.*`.

pub mod android_key;
pub mod none;
pub mod packed;
pub mod tpm;

use ciborium::value::Value;

use super::WebAuthnError;
use super::cbor;

/// Parsed attestationObject (W3C WebAuthn L2 §6.5).
#[derive(Debug, Clone)]
pub struct AttestationObject {
    pub fmt: String,
    pub auth_data_raw: Vec<u8>,
    pub statement: AttestationStatement,
}

/// Tagged variant — discriminated by `fmt`.
#[derive(Debug, Clone)]
pub enum AttestationStatement {
    None,
    Packed(packed::PackedAttStmt),
    Tpm(tpm::TpmAttStmt),
    AndroidKey(android_key::AndroidKeyAttStmt),
    Unsupported { fmt: String, raw: Value },
}

/// Parse the attestationObject CBOR map.
///
/// Port of webauthn4j `AttestationObjectConverter#convert`.
pub fn parse(raw: &[u8]) -> Result<AttestationObject, WebAuthnError> {
    let v = cbor::decode(raw)?;
    let fmt = cbor::map_get_str(&v, "fmt")
        .ok_or_else(|| WebAuthnError::Attestation("missing fmt".into()))?;
    let fmt = cbor::as_text(fmt)?.to_string();
    let auth_data = cbor::map_get_str(&v, "authData")
        .ok_or_else(|| WebAuthnError::Attestation("missing authData".into()))?;
    let auth_data_raw = cbor::as_bytes(auth_data)?.to_vec();
    let stmt = cbor::map_get_str(&v, "attStmt")
        .ok_or_else(|| WebAuthnError::Attestation("missing attStmt".into()))?;

    let statement = match fmt.as_str() {
        "none" => {
            none::parse(stmt)?;
            AttestationStatement::None
        }
        "packed" => AttestationStatement::Packed(packed::parse(stmt)?),
        "tpm" => AttestationStatement::Tpm(tpm::parse(stmt)?),
        "android-key" => AttestationStatement::AndroidKey(android_key::parse(stmt)?),
        other => AttestationStatement::Unsupported {
            fmt: other.to_string(),
            raw: stmt.clone(),
        },
    };
    Ok(AttestationObject {
        fmt,
        auth_data_raw,
        statement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciborium::value::Value;

    fn build_attestation(fmt: &str, auth_data: &[u8], stmt: Value) -> Vec<u8> {
        let m = Value::Map(vec![
            (Value::Text("fmt".into()), Value::Text(fmt.into())),
            (
                Value::Text("authData".into()),
                Value::Bytes(auth_data.to_vec()),
            ),
            (Value::Text("attStmt".into()), stmt),
        ]);
        cbor::encode(&m).unwrap()
    }

    #[test]
    fn parse_none_attestation() {
        let raw = build_attestation("none", &[0x01], Value::Map(vec![]));
        let obj = parse(&raw).unwrap();
        assert_eq!(obj.fmt, "none");
        assert!(matches!(obj.statement, AttestationStatement::None));
        assert_eq!(obj.auth_data_raw, vec![0x01]);
    }

    #[test]
    fn parse_unknown_fmt_falls_through() {
        let raw = build_attestation("apple", &[0], Value::Map(vec![]));
        let obj = parse(&raw).unwrap();
        assert!(matches!(
            obj.statement,
            AttestationStatement::Unsupported { fmt, .. } if fmt == "apple"
        ));
    }

    #[test]
    fn parse_missing_fmt_errors() {
        let m = Value::Map(vec![(
            Value::Text("authData".into()),
            Value::Bytes(vec![1]),
        )]);
        let raw = cbor::encode(&m).unwrap();
        assert!(parse(&raw).is_err());
    }
}

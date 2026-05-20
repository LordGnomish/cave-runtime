// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// Minimal CBOR decoder for attestation objects.
//
// webauthn4j uses Jackson's CBOR module. We delegate to the `ciborium` crate
// (BSD-2-Clause, ZX-licensed by user), and surface a tiny façade that
// returns a `ciborium::value::Value` tree. Higher modules (`cose`,
// `attestation`) match on that tree directly.

use ciborium::value::Value;
use std::io::Cursor;

use super::WebAuthnError;

/// Decode a single CBOR item from a byte slice.
///
/// Mirrors `AttestationObjectConverter#readAttestationObject` (webauthn4j)
/// which calls `ObjectMapper#readValue`.
pub fn decode(bytes: &[u8]) -> Result<Value, WebAuthnError> {
    ciborium::de::from_reader(Cursor::new(bytes)).map_err(|e| WebAuthnError::Cbor(e.to_string()))
}

/// Encode a CBOR value to bytes. Used by tests and by the credential-public-
/// key round-trip path in `cose`.
pub fn encode(v: &Value) -> Result<Vec<u8>, WebAuthnError> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(v, &mut buf).map_err(|e| WebAuthnError::Cbor(e.to_string()))?;
    Ok(buf)
}

/// Look up a key in a CBOR map. The CBOR maps used by WebAuthn are integer-
/// keyed (e.g. COSE_Key) **or** string-keyed (e.g. attestation-object outer
/// map). This helper accepts either.
///
/// Mirrors webauthn4j `CborMapHelper`.
pub fn map_get<'a>(map: &'a Value, key: &Value) -> Option<&'a Value> {
    let Value::Map(entries) = map else {
        return None;
    };
    entries
        .iter()
        .find_map(|(k, v)| if k == key { Some(v) } else { None })
}

/// Look up a string-keyed entry — convenience for the attestation-object outer map.
pub fn map_get_str<'a>(map: &'a Value, key: &str) -> Option<&'a Value> {
    map_get(map, &Value::Text(key.to_string()))
}

/// Look up an integer-keyed entry — convenience for COSE_Key.
pub fn map_get_int<'a>(map: &'a Value, key: i64) -> Option<&'a Value> {
    map_get(map, &Value::Integer(key.into()))
}

/// Extract a `Vec<u8>` from a CBOR `Bytes` value.
pub fn as_bytes(v: &Value) -> Result<&[u8], WebAuthnError> {
    match v {
        Value::Bytes(b) => Ok(b),
        _ => Err(WebAuthnError::Cbor("expected bytes".into())),
    }
}

/// Extract an i64 from a CBOR `Integer` value.
pub fn as_i64(v: &Value) -> Result<i64, WebAuthnError> {
    match v {
        Value::Integer(i) => i128::from(*i)
            .try_into()
            .map_err(|_| WebAuthnError::Cbor("integer out of range".into())),
        _ => Err(WebAuthnError::Cbor("expected integer".into())),
    }
}

/// Extract a UTF-8 string from a CBOR `Text` value.
pub fn as_text(v: &Value) -> Result<&str, WebAuthnError> {
    match v {
        Value::Text(s) => Ok(s),
        _ => Err(WebAuthnError::Cbor("expected text".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_simple_map_roundtrip() {
        // CBOR: { "fmt": "none", "authData": h'01', "attStmt": {} }
        let original = Value::Map(vec![
            (Value::Text("fmt".into()), Value::Text("none".into())),
            (Value::Text("authData".into()), Value::Bytes(vec![0x01])),
            (Value::Text("attStmt".into()), Value::Map(vec![])),
        ]);
        let encoded = encode(&original).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn map_get_str_finds_string_key() {
        let m = Value::Map(vec![(
            Value::Text("fmt".into()),
            Value::Text("packed".into()),
        )]);
        assert_eq!(map_get_str(&m, "fmt"), Some(&Value::Text("packed".into())));
        assert_eq!(map_get_str(&m, "missing"), None);
    }

    #[test]
    fn map_get_int_finds_integer_key() {
        let m = Value::Map(vec![(
            Value::Integer(1i64.into()),
            Value::Integer(2i64.into()),
        )]);
        assert_eq!(map_get_int(&m, 1), Some(&Value::Integer(2i64.into())));
        assert_eq!(map_get_int(&m, 2), None);
    }

    #[test]
    fn as_helpers_reject_wrong_type() {
        let t = Value::Text("hi".into());
        assert!(as_bytes(&t).is_err());
        assert!(as_i64(&t).is_err());
        let b = Value::Bytes(vec![1, 2, 3]);
        assert!(as_text(&b).is_err());
    }

    #[test]
    fn malformed_cbor_returns_error() {
        // 0xff alone is "break" with no enclosing context — invalid item.
        let err = decode(&[0xff]);
        assert!(err.is_err());
    }
}

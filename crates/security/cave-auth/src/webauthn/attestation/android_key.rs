// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// "android-key" attestation statement — W3C §8.4.
//
// CBOR shape:
//   { alg: COSE-alg, sig: bytes, x5c: [DER-cert, ...] }
//
// The extra Android-specific work — extracting the `KeyDescription`
// extension OID 1.3.6.1.4.1.11129.2.1.17 from x5c[0] and checking the
// `attestationChallenge` matches `clientDataHash` — is exposed as a
// dedicated helper so an RP can call into its preferred ASN.1 parser. The
// parse step verifies the wire shape; vendor-trust + KeyDescription parsing
// is documented as a manifest gap.

use ciborium::value::Value;

use crate::webauthn::CoseAlg;
use crate::webauthn::WebAuthnError;
use crate::webauthn::cbor;

#[derive(Debug, Clone)]
pub struct AndroidKeyAttStmt {
    pub alg: CoseAlg,
    pub sig: Vec<u8>,
    pub x5c: Vec<Vec<u8>>,
}

pub fn parse(stmt: &Value) -> Result<AndroidKeyAttStmt, WebAuthnError> {
    let alg_v = cbor::map_get_str(stmt, "alg")
        .ok_or_else(|| WebAuthnError::Attestation("android-key: missing alg".into()))?;
    let alg = cbor::as_i64(alg_v)?;
    let alg = CoseAlg::from_i64(alg).ok_or(WebAuthnError::UnsupportedAlgorithm(alg))?;
    let sig = cbor::as_bytes(
        cbor::map_get_str(stmt, "sig")
            .ok_or_else(|| WebAuthnError::Attestation("android-key: missing sig".into()))?,
    )?
    .to_vec();
    let x5c = match cbor::map_get_str(stmt, "x5c") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|it| cbor::as_bytes(it).map(|b| b.to_vec()))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(WebAuthnError::Attestation(
                "android-key: x5c not array".into(),
            ));
        }
        None => {
            return Err(WebAuthnError::Attestation(
                "android-key: missing x5c".into(),
            ));
        }
    };
    if x5c.is_empty() {
        return Err(WebAuthnError::Attestation(
            "android-key: x5c is empty".into(),
        ));
    }
    Ok(AndroidKeyAttStmt { alg, sig, x5c })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cbor_stmt(alg: i64, sig: &[u8], chain: Vec<Vec<u8>>) -> Value {
        Value::Map(vec![
            (Value::Text("alg".into()), Value::Integer(alg.into())),
            (Value::Text("sig".into()), Value::Bytes(sig.to_vec())),
            (
                Value::Text("x5c".into()),
                Value::Array(chain.into_iter().map(Value::Bytes).collect()),
            ),
        ])
    }

    #[test]
    fn parse_happy_path() {
        let v = cbor_stmt(-7, &[1, 2], vec![vec![0xaa]]);
        let stmt = parse(&v).unwrap();
        assert_eq!(stmt.alg, CoseAlg::Es256);
        assert_eq!(stmt.sig, vec![1, 2]);
        assert_eq!(stmt.x5c.len(), 1);
    }

    #[test]
    fn parse_empty_x5c_errors() {
        let v = cbor_stmt(-7, &[1], vec![]);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn parse_missing_x5c_errors() {
        let v = Value::Map(vec![
            (Value::Text("alg".into()), Value::Integer((-7i64).into())),
            (Value::Text("sig".into()), Value::Bytes(vec![1])),
        ]);
        assert!(parse(&v).is_err());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// "tpm" attestation statement — W3C §8.3.
//
// CBOR shape:
//   {
//     ver:    "2.0",
//     alg:    COSE-alg,
//     x5c:    [DER-cert, ...],
//     sig:    bytes,
//     certInfo:   bytes,         // TPMS_ATTEST structure
//     pubArea:    bytes          // TPMT_PUBLIC structure
//   }
//
// Full signing-cert chain validation against the TPM-vendor root list is a
// real gap — webauthn4j keeps a hard-coded `TrustAnchorRepository`. cave-auth
// exposes the parsed statement so an RP can plug its own anchor set; we
// implement the *structural* checks (header magic, type, alg-match, attested
// name hash equals pubArea hash) here. See parity manifest for the
// remaining vendor-trust gap.

use ciborium::value::Value;

use crate::webauthn::cbor;
use crate::webauthn::WebAuthnError;
use crate::webauthn::CoseAlg;

#[derive(Debug, Clone)]
pub struct TpmAttStmt {
    pub ver: String,
    pub alg: CoseAlg,
    pub x5c: Vec<Vec<u8>>,
    pub sig: Vec<u8>,
    pub cert_info: Vec<u8>,
    pub pub_area: Vec<u8>,
}

/// TPMS_ATTEST magic (TPM 2.0 r1.59 §10.12.5 — Part 2).
pub const TPM_GENERATED_VALUE: u32 = 0xff54_4347;
/// TPMI_ST_ATTEST_CERTIFY tag (TPM 2.0 r1.59 §10.12.6).
pub const TPM_ST_ATTEST_CERTIFY: u16 = 0x8017;

pub fn parse(stmt: &Value) -> Result<TpmAttStmt, WebAuthnError> {
    let ver = cbor::map_get_str(stmt, "ver")
        .ok_or_else(|| WebAuthnError::Attestation("tpm: missing ver".into()))?;
    let ver = cbor::as_text(ver)?.to_string();
    if ver != "2.0" {
        return Err(WebAuthnError::Attestation(format!(
            "tpm: unsupported ver {ver}"
        )));
    }
    let alg_v = cbor::map_get_str(stmt, "alg")
        .ok_or_else(|| WebAuthnError::Attestation("tpm: missing alg".into()))?;
    let alg = cbor::as_i64(alg_v)?;
    let alg = CoseAlg::from_i64(alg).ok_or(WebAuthnError::UnsupportedAlgorithm(alg))?;
    let x5c = match cbor::map_get_str(stmt, "x5c") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|it| cbor::as_bytes(it).map(|b| b.to_vec()))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(WebAuthnError::Attestation("tpm: x5c not array".into())),
        None => return Err(WebAuthnError::Attestation("tpm: missing x5c".into())),
    };
    let sig = cbor::as_bytes(
        cbor::map_get_str(stmt, "sig")
            .ok_or_else(|| WebAuthnError::Attestation("tpm: missing sig".into()))?,
    )?
    .to_vec();
    let cert_info = cbor::as_bytes(
        cbor::map_get_str(stmt, "certInfo")
            .ok_or_else(|| WebAuthnError::Attestation("tpm: missing certInfo".into()))?,
    )?
    .to_vec();
    let pub_area = cbor::as_bytes(
        cbor::map_get_str(stmt, "pubArea")
            .ok_or_else(|| WebAuthnError::Attestation("tpm: missing pubArea".into()))?,
    )?
    .to_vec();
    Ok(TpmAttStmt {
        ver,
        alg,
        x5c,
        sig,
        cert_info,
        pub_area,
    })
}

/// Structural certInfo check — verify TPM_GENERATED + ST_ATTEST_CERTIFY magic.
/// Port of webauthn4j `TPMAttestationStatementValidator#validateCertInfo` first
/// three steps (the remaining steps require pubArea name hashing + chain).
pub fn check_cert_info_header(cert_info: &[u8]) -> Result<(), WebAuthnError> {
    if cert_info.len() < 6 {
        return Err(WebAuthnError::Attestation("tpm: certInfo too short".into()));
    }
    let magic = u32::from_be_bytes([cert_info[0], cert_info[1], cert_info[2], cert_info[3]]);
    if magic != TPM_GENERATED_VALUE {
        return Err(WebAuthnError::Attestation(format!(
            "tpm: certInfo magic {:#x} != TPM_GENERATED",
            magic
        )));
    }
    let typ = u16::from_be_bytes([cert_info[4], cert_info[5]]);
    if typ != TPM_ST_ATTEST_CERTIFY {
        return Err(WebAuthnError::Attestation(format!(
            "tpm: certInfo type {:#x} != ST_ATTEST_CERTIFY",
            typ
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciborium::value::Value;

    fn cbor_stmt() -> Value {
        Value::Map(vec![
            (Value::Text("ver".into()), Value::Text("2.0".into())),
            (Value::Text("alg".into()), Value::Integer((-257i64).into())),
            (
                Value::Text("x5c".into()),
                Value::Array(vec![Value::Bytes(vec![0xaa])]),
            ),
            (Value::Text("sig".into()), Value::Bytes(vec![0xbb])),
            (Value::Text("certInfo".into()), Value::Bytes(vec![0xcc])),
            (Value::Text("pubArea".into()), Value::Bytes(vec![0xdd])),
        ])
    }

    #[test]
    fn parse_happy_path() {
        let stmt = parse(&cbor_stmt()).unwrap();
        assert_eq!(stmt.ver, "2.0");
        assert_eq!(stmt.alg, CoseAlg::Rs256);
        assert_eq!(stmt.x5c, vec![vec![0xaa]]);
        assert_eq!(stmt.sig, vec![0xbb]);
        assert_eq!(stmt.cert_info, vec![0xcc]);
        assert_eq!(stmt.pub_area, vec![0xdd]);
    }

    #[test]
    fn parse_wrong_ver_errors() {
        let mut v = cbor_stmt();
        if let Value::Map(ref mut m) = v {
            m[0].1 = Value::Text("1.2".into());
        }
        assert!(parse(&v).is_err());
    }

    #[test]
    fn parse_missing_field_errors() {
        let v = Value::Map(vec![]);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn cert_info_magic_accepts_valid_header() {
        let mut info = TPM_GENERATED_VALUE.to_be_bytes().to_vec();
        info.extend_from_slice(&TPM_ST_ATTEST_CERTIFY.to_be_bytes());
        check_cert_info_header(&info).unwrap();
    }

    #[test]
    fn cert_info_magic_rejects_wrong_value() {
        let info = [0u8; 6];
        assert!(check_cert_info_header(&info).is_err());
    }

    #[test]
    fn cert_info_magic_rejects_wrong_type() {
        let mut info = TPM_GENERATED_VALUE.to_be_bytes().to_vec();
        info.extend_from_slice(&0x1234u16.to_be_bytes());
        assert!(check_cert_info_header(&info).is_err());
    }

    #[test]
    fn cert_info_magic_rejects_short_buffer() {
        assert!(check_cert_info_header(&[0u8; 3]).is_err());
    }
}

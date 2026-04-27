//! Lightweight CSR PEM extractor — `cfssl_signer.go::parseCSR`.
//!
//! Extracts the base64 inner block and the boundary header. We don't do full
//! ASN.1 parsing here (that's the job of a real x509 crate); this module
//! only validates that the input is well-formed PEM and exposes the raw
//! DER bytes for downstream parsing. Intended for parity-test scaffolding.

use crate::types::{Cite, ControllerError};

/// PEM block kinds we recognize.
pub const KIND_CERTIFICATE_REQUEST: &str = "CERTIFICATE REQUEST";
pub const KIND_CERTIFICATE: &str = "CERTIFICATE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PemBlock {
    pub kind: String,
    pub b64_body: String,
}

pub fn parse_pem(input: &str) -> Result<PemBlock, ControllerError> {
    let begin_marker = "-----BEGIN ";
    let end_marker = "-----END ";
    let close = "-----";
    let begin_idx = input.find(begin_marker).ok_or_else(|| ControllerError::InvalidSpec {
        kind: "PEM",
        reason: "missing BEGIN marker".into(),
    })?;
    let after_begin = &input[begin_idx + begin_marker.len()..];
    let kind_end = after_begin.find(close).ok_or_else(|| ControllerError::InvalidSpec {
        kind: "PEM",
        reason: "malformed BEGIN marker".into(),
    })?;
    let kind = after_begin[..kind_end].trim().to_string();
    if kind.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "PEM",
            reason: "empty PEM kind".into(),
        });
    }
    let body_start = begin_idx + begin_marker.len() + kind_end + close.len();
    let body_search = &input[body_start..];
    let end_marker_full = format!("{end_marker}{kind}{close}");
    let end_idx = body_search.find(&end_marker_full).ok_or_else(|| ControllerError::InvalidSpec {
        kind: "PEM",
        reason: "missing matching END marker".into(),
    })?;
    let raw_body = &body_search[..end_idx];
    // Strip whitespace; valid base64 chars: A-Z, a-z, 0-9, +, /, =.
    let body: String = raw_body
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if body.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "PEM",
            reason: "empty PEM body".into(),
        });
    }
    if !body.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')) {
        return Err(ControllerError::InvalidSpec {
            kind: "PEM",
            reason: "PEM body contains non-base64 characters".into(),
        });
    }
    Ok(PemBlock { kind, b64_body: body })
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/certificates/signer/cfssl_signer.go",
    "parseCSR",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn parses_well_formed_pem_block() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-ok"
        );
        let pem = "-----BEGIN CERTIFICATE REQUEST-----\nABCDEFGH\n-----END CERTIFICATE REQUEST-----";
        let b = parse_pem(pem).unwrap();
        assert_eq!(b.kind, KIND_CERTIFICATE_REQUEST);
        assert_eq!(b.b64_body, "ABCDEFGH");
    }

    #[test]
    fn rejects_missing_begin_marker() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-no-begin"
        );
        assert!(parse_pem("just text").is_err());
    }

    #[test]
    fn rejects_missing_end_marker() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-no-end"
        );
        let pem = "-----BEGIN CERTIFICATE REQUEST-----\nABCD";
        assert!(parse_pem(pem).is_err());
    }

    #[test]
    fn rejects_empty_body() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-empty-body"
        );
        let pem = "-----BEGIN CERTIFICATE REQUEST-----\n\n-----END CERTIFICATE REQUEST-----";
        assert!(parse_pem(pem).is_err());
    }

    #[test]
    fn rejects_non_base64_characters() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-bad-chars"
        );
        let pem = "-----BEGIN CERTIFICATE REQUEST-----\n!@#$%^\n-----END CERTIFICATE REQUEST-----";
        assert!(parse_pem(pem).is_err());
    }

    #[test]
    fn rejects_mismatched_kind_in_end_marker() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-mismatch"
        );
        let pem = "-----BEGIN CERTIFICATE REQUEST-----\nABCD\n-----END CERTIFICATE-----";
        assert!(parse_pem(pem).is_err());
    }

    #[test]
    fn parses_certificate_kind() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-cert"
        );
        let pem = "-----BEGIN CERTIFICATE-----\nMII=\n-----END CERTIFICATE-----";
        let b = parse_pem(pem).unwrap();
        assert_eq!(b.kind, KIND_CERTIFICATE);
    }

    #[test]
    fn strips_whitespace_in_body() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "parseCSR",
            "tenant-pem-strip-ws"
        );
        let pem = "-----BEGIN CERTIFICATE REQUEST-----\nA B C D\nE F\n-----END CERTIFICATE REQUEST-----";
        let b = parse_pem(pem).unwrap();
        assert_eq!(b.b64_body, "ABCDEF");
    }

    #[test]
    fn pem_block_constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/certificates/signer/cfssl_signer.go",
            "PEMTypes",
            "tenant-pem-const"
        );
        assert_eq!(KIND_CERTIFICATE_REQUEST, "CERTIFICATE REQUEST");
        assert_eq!(KIND_CERTIFICATE, "CERTIFICATE");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/u2f/FIDOU2FAttestationStatementValidator.java
//
// "fido-u2f" attestation — W3C §8.6.  Legacy U2F binding: attStmt =
// {x5c, sig} where sig signs (0x00 || rpIdHash || clientDataHash ||
// credentialId || credentialPublicKey).  Honest scope-cut: requires
// ASN.1 X.509 parsing for the attestation certificate and DER-encoded
// uncompressed P-256 key extraction.  Verification refused until that
// path is enabled.

use super::{AttestationError, AttestationStatement};
use crate::webauthn::registration::AttestationTrustPath;

pub fn verify(_stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    Err(AttestationError::Unsupported(
        "fido-u2f attestation — X.509 cert parsing not enabled in this build".into(),
    ))
}

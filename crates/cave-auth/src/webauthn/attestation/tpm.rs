// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/tpm/TPMAttestationStatementValidator.java
//
// "tpm" attestation — W3C §8.3.  attStmt = {ver, alg, x5c, sig, certInfo,
// pubArea}.  certInfo is a TPMS_ATTEST structure (TPM 2.0 Part 2);
// pubArea is TPMT_PUBLIC.  Real verification requires parsing both
// TCG structures and walking the TPM EK chain.  Honest scope-cut.

use super::{AttestationError, AttestationStatement};
use crate::webauthn::registration::AttestationTrustPath;

pub fn verify(_stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    Err(AttestationError::Unsupported(
        "tpm attestation — TPMS_ATTEST/TPMT_PUBLIC parser not enabled in this build".into(),
    ))
}

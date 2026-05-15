// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/androidkey/AndroidKeyAttestationStatementValidator.java
//
// "android-key" attestation — W3C §8.4.  attStmt = {alg, sig, x5c}.
// Validation requires parsing the Android Key Attestation extension
// (1.3.6.1.4.1.11129.2.1.17) ASN.1 sequence to check attestationChallenge
// equals clientDataHash plus the KeyDescription bootloader+softwareEnforced
// chains.  Honest scope-cut.

use super::{AttestationError, AttestationStatement};
use crate::webauthn::registration::AttestationTrustPath;

pub fn verify(_stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    Err(AttestationError::Unsupported(
        "android-key attestation — KeyDescription ASN.1 parser not enabled in this build".into(),
    ))
}

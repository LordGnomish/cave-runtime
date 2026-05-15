// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/androidsafetynet/AndroidSafetyNetAttestationStatementValidator.java
//
// "android-safetynet" attestation — W3C §8.5.  attStmt = {ver, response}.
// The response is a JWS produced by Google SafetyNet.  Deprecated by
// Google in favour of Play Integrity, but webauthn4j still ships a
// verifier.  Honest scope-cut: JWS + Google Root CA pinning not yet
// wired.

use super::{AttestationError, AttestationStatement};
use crate::webauthn::registration::AttestationTrustPath;

pub fn verify(_stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    Err(AttestationError::Unsupported(
        "android-safetynet attestation — JWS chain not enabled in this build".into(),
    ))
}

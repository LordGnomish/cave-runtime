// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/apple/AppleAnonymousAttestationStatementValidator.java
//
// "apple" anonymous attestation — Apple devices.  attStmt = {x5c}.
// Verification requires the Apple WebAuthn Root CA + nonce extension
// (1.2.840.113635.100.8.2) parsing.  Honest scope-cut: X.509 + ASN.1
// stack not yet wired.

use super::{AttestationError, AttestationStatement};
use crate::webauthn::registration::AttestationTrustPath;

pub fn verify(_stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    Err(AttestationError::Unsupported(
        "apple attestation — Apple Root CA chain not enabled in this build".into(),
    ))
}

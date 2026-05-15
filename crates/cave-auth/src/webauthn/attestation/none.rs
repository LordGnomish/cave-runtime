// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/none/NoneAttestationStatementValidator.java
//
// "none" attestation — W3C §8.7.  attStmt MUST be an empty CBOR map.

use super::{AttestationError, AttestationStatement};
use crate::webauthn::registration::AttestationTrustPath;

pub fn verify(stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    match &stmt.att_stmt {
        ciborium::Value::Map(m) if m.is_empty() => Ok(AttestationTrustPath::None),
        ciborium::Value::Map(_) => Err(AttestationError::BadStatement),
        _ => Err(AttestationError::BadStatement),
    }
}

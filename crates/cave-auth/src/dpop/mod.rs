// SPDX-License-Identifier: AGPL-3.0-or-later
//
// DPoP (Demonstration of Proof-of-Possession at the Application Layer) —
// RFC 9449.
//
// Upstream parity:
//   - keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38  (v22.0.0)
//     services/src/main/java/org/keycloak/protocol/oidc/grants/DPoPHandler.java
//     services/src/main/java/org/keycloak/crypto/DPoPProofValidator.java
//   - RFC 9449 — OAuth 2.0 Demonstrating Proof of Possession (DPoP)
//   - RFC 7638 — JWK Thumbprint (used to compute the `jkt` confirmation claim)
//
// Sub-modules:
//   - [`header`]     — parse the `DPoP` HTTP header (compact JWS)
//   - [`proof`]      — DPoP proof JWT claim model + signature verification
//   - [`nonce`]      — DPoP-nonce policy & `WWW-Authenticate: DPoP error="use_dpop_nonce"` challenges
//   - [`thumbprint`] — RFC 7638 JWK thumbprint (= `cnf.jkt`)

pub mod header;
pub mod nonce;
pub mod proof;
pub mod thumbprint;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DpopError {
    #[error("invalid DPoP header structure: {0}")]
    Header(&'static str),
    #[error("invalid DPoP proof JWT: {0}")]
    Proof(&'static str),
    #[error("invalid signature")]
    BadSignature,
    #[error("unsupported alg: {0}")]
    UnsupportedAlg(String),
    #[error("htm mismatch (expected {expected}, got {got})")]
    HtmMismatch { expected: String, got: String },
    #[error("htu mismatch (expected {expected}, got {got})")]
    HtuMismatch { expected: String, got: String },
    #[error("iat out of window")]
    IatOutOfWindow,
    #[error("missing jti")]
    MissingJti,
    #[error("replay: jti seen before")]
    Replay,
    #[error("nonce mismatch")]
    NonceMismatch,
    #[error("ath mismatch")]
    AthMismatch,
    #[error("base64 decode: {0}")]
    Base64(String),
    #[error("json: {0}")]
    Json(String),
}

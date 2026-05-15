// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/+ RFC 9449 (DPoP)
//
//! DPoP — Demonstrating Proof-of-Possession at the Application Layer (RFC 9449).
//!
//! ## Surface
//!
//! - [`proof::DpopProof`] — parses & validates a `DPoP` HTTP header value.
//! - [`verify::verify_proof`] — full verification against an access-token's `cnf` claim.
//! - [`binding::jkt_thumbprint`] — RFC 7638 JWK Thumbprint of a public JWK.
//! - [`replay_guard::ReplayGuard`] — in-memory `jti` window enforced per RFC 9449 §11.1.
//!
//! ## Honest deferrals
//!
//! - Distributed `jti` replay-guard (Redis-backed): out of scope, the in-memory
//!   guard is sufficient for single-node verifiers and is the upstream test target.
//! - DPoP nonce challenge (RFC 9449 §8): not implemented — `nonce` claim is read
//!   but not generated/validated against a server-issued nonce.
//! - Only `ES256` and `RS256` are accepted as proof-signature algorithms.

pub mod binding;
pub mod proof;
pub mod replay_guard;
pub mod verify;

pub use binding::{jkt_thumbprint, Jwk};
pub use proof::{DpopHeader, DpopPayload, DpopProof, DpopProofError};
pub use replay_guard::ReplayGuard;
pub use verify::{verify_proof, DpopVerifyError};

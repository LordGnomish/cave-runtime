// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/encryption/+ RFC 7516/7518 (JWE/JWA)
//
//! JSON Web Encryption (RFC 7516) — encrypted tokens.
//!
//! Algorithms (a strict subset of RFC 7518 that real-world OIDC IdPs deploy):
//!
//! | `alg` (key-management) | `enc` (content) |
//! |------------------------|-----------------|
//! | `RSA-OAEP`             | `A256GCM`       |
//! | `RSA-OAEP`             | `A128CBC-HS256` |
//! | `ECDH-ES+A256KW`       | `A256GCM`       |
//! | `ECDH-ES+A256KW`       | `A128CBC-HS256` |
//!
//! Honest deferrals:
//!   - No JWE-JSON-Serialization (RFC 7516 §7.2) — compact only.
//!   - No `zip` (deflate) compression — wire plaintext as-is.
//!   - No multi-recipient — single `recipient` per token.

pub mod content_encryption;
pub mod header;
pub mod key_agreement;
pub mod serialize;

pub use content_encryption::{ContentEncAlg, ContentEncError};
pub use header::{JweHeader, KeyAgreementAlg};
pub use key_agreement::{KeyAgreementError, WrappedKey};
pub use serialize::{JweCompact, JweError, compact_decode, compact_encode};

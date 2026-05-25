// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SAML 2.0 broker — port of Keycloak's `saml-core` + the
//! `services/.../protocol/saml/` endpoint set. cave-auth is
//! primarily OIDC; this module adds the second federation
//! protocol enterprise customers expect.
//!
//! ## What this module covers
//!
//! * [`authn_request`] — SAML 2.0 `<samlp:AuthnRequest>` builder
//!   and parser (the SP → IdP "please authenticate this user"
//!   message).
//! * [`response`]      — `<samlp:Response>` containing a single
//!   `<saml:Assertion>` (the IdP → SP "here is the
//!   authenticated subject" message).
//! * [`metadata`]      — `<md:EntityDescriptor>` for both SP and
//!   IdP roles. The metadata XML two parties exchange before
//!   any flow can run.
//! * [`binding`]       — HTTP-Redirect (deflate + base64 + URL)
//!   and HTTP-POST (base64) bindings. The two transport encodings
//!   any SAML deployment hits in practice.
//! * [`signature`]     — RSA-SHA256 sign + verify over the
//!   pre-canonicalized bytes of a SAML message. XML
//!   canonicalization (`exc-c14n`) is a separate concern — see
//!   the limitation note on [`signature::SignedDocument`]. The
//!   unified [`signature::sign`] / [`signature::verify`] entry
//!   points dispatch by [`signature::Algorithm`] across RSA-SHA256
//!   plus the three ECDSA variants from [`signing_ecdsa`].
//! * [`signing_ecdsa`] — ECDSA-SHA256/384/512 over NIST P-256 /
//!   P-384 / P-521 (XMLDSig 2.0, RFC 4051 §2.2.3). Signatures are
//!   raw R||S concatenation, padded to curve scalar size; PKCS#8
//!   PEM round-trips supported on every curve.
//! * [`broker`]        — SP-initiated and IdP-initiated flow
//!   state machines. Holds in-flight request state keyed by
//!   `RequestID`.
//!
//! ## Honest limitations
//!
//! * Full XML canonicalization is **not** implemented. The
//!   sign / verify path operates over pre-canonicalized bytes
//!   the caller supplies. A real production IdP integration
//!   still needs a c14n implementation; the framework is in
//!   place but the canonicalization step is intentionally
//!   pluggable.
//! * Encrypted Assertions (`<saml:EncryptedAssertion>`) are
//!   parsed-but-not-decrypted. Decryption requires XML-Enc
//!   key-transport which is its own RFC. Tracked, not landed.
//! * Artifact Resolution binding (back-channel) is out of scope —
//!   the two front-channel bindings (Redirect + POST) cover
//!   every IdP cave customers federate with.

pub mod authn_request;
pub mod binding;
pub mod broker;
pub mod canonicalization;
pub mod metadata;
pub mod response;
pub mod signature;
pub mod signing_ecdsa;

// ── A1 mission additions (Keycloak v22.0.0 SAML broker port) ────────────────
// Appended-only — never edit the originals above.
pub mod assertion;
pub mod bindings;
pub mod name_id;

#[cfg(test)]
mod tests_a1;

use thiserror::Error;

/// Errors the SAML surface can surface. Modelled on Keycloak's
/// `SAMLProtocolException` — the cases callers actually need to
/// branch on (malformed XML, invalid signature, missing fields,
/// stale message) plus a catch-all.
#[derive(Debug, Error)]
pub enum SamlError {
    /// XML couldn't be parsed (malformed, truncated, bad
    /// namespace).
    #[error("SAML XML parse error: {0}")]
    Parse(String),
    /// XML parsed but a required element / attribute was
    /// missing (e.g. `Response` with no `Assertion`).
    #[error("SAML missing required field: {0}")]
    MissingField(String),
    /// Signature verification failed — wrong key, tampered
    /// body, or unsupported algorithm.
    #[error("SAML signature invalid: {0}")]
    InvalidSignature(String),
    /// Message arrived outside its NotBefore / NotOnOrAfter
    /// window, or its `IssueInstant` is too far skewed.
    #[error("SAML message expired or not yet valid")]
    Expired,
    /// Request / Response `Destination` doesn't match the
    /// receiving endpoint, or the `InResponseTo` doesn't match
    /// any in-flight request the broker is tracking.
    #[error("SAML message addressed wrong: {0}")]
    WrongDestination(String),
    /// HTTP-Redirect deflate or base64 decode failed.
    #[error("SAML binding decode error: {0}")]
    Binding(String),
    /// Catch-all for cases that aren't worth a distinct
    /// variant.
    #[error("SAML error: {0}")]
    Other(String),
}

/// `NameID` formats Keycloak supports. The string constants
/// match the SAML 2.0 spec verbatim (`urn:oasis:names:tc:SAML:...`)
/// — preserved so wire-format round-trips byte-for-byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameIdFormat {
    /// Email-address-shaped subject identifier.
    EmailAddress,
    /// Persistent opaque identifier — stable per-SP across
    /// sessions.
    Persistent,
    /// Transient opaque identifier — fresh per session.
    Transient,
    /// Free-form, no structure assumed.
    Unspecified,
}

impl NameIdFormat {
    /// SAML 2.0 URN for this format.
    pub fn as_urn(self) -> &'static str {
        match self {
            NameIdFormat::EmailAddress => "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
            NameIdFormat::Persistent => "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
            NameIdFormat::Transient => "urn:oasis:names:tc:SAML:2.0:nameid-format:transient",
            NameIdFormat::Unspecified => "urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified",
        }
    }

    /// Inverse of `as_urn` — returns `None` for unknown URNs so
    /// the parser can fall back to `Unspecified`.
    pub fn from_urn(s: &str) -> Option<Self> {
        match s {
            "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress" => {
                Some(NameIdFormat::EmailAddress)
            }
            "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent" => {
                Some(NameIdFormat::Persistent)
            }
            "urn:oasis:names:tc:SAML:2.0:nameid-format:transient" => Some(NameIdFormat::Transient),
            "urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified" => {
                Some(NameIdFormat::Unspecified)
            }
            _ => None,
        }
    }
}

/// Subject + attribute payload extracted from a verified SAML
/// Assertion. The thing the broker actually returns up to the
/// auth_middleware layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamlSubject {
    /// `<saml:NameID>` of the authenticated principal.
    pub name_id: String,
    /// Format of `name_id`.
    pub name_id_format: NameIdFormat,
    /// IdP entity ID that issued the Assertion.
    pub issuer: String,
    /// Flat attribute-statement map. SAML attributes can be
    /// multi-valued, but cave-auth flattens to first-value;
    /// real IdPs almost always send single values for the
    /// claims OIDC equivalents would carry (email, name,
    /// groups list).
    pub attributes: std::collections::BTreeMap<String, Vec<String>>,
    /// SAML session index — the IdP's session reference, used
    /// for single-logout correlation.
    pub session_index: Option<String>,
}

/// XML namespaces every SAML message uses. Pulled out so writers
/// and parsers agree on the canonical prefix; Keycloak emits the
/// same `saml:` / `samlp:` / `md:` / `ds:` prefixes.
pub mod ns {
    pub const SAML_ASSERTION: &str = "urn:oasis:names:tc:SAML:2.0:assertion";
    pub const SAML_PROTOCOL: &str = "urn:oasis:names:tc:SAML:2.0:protocol";
    pub const SAML_METADATA: &str = "urn:oasis:names:tc:SAML:2.0:metadata";
    pub const XML_DSIG: &str = "http://www.w3.org/2000/09/xmldsig#";
    pub const XML_ENC: &str = "http://www.w3.org/2001/04/xmlenc#";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nameid_format_roundtrips_through_urn() {
        for f in [
            NameIdFormat::EmailAddress,
            NameIdFormat::Persistent,
            NameIdFormat::Transient,
            NameIdFormat::Unspecified,
        ] {
            assert_eq!(NameIdFormat::from_urn(f.as_urn()), Some(f));
        }
    }

    #[test]
    fn nameid_format_unknown_urn_is_none() {
        assert!(NameIdFormat::from_urn("urn:other:something").is_none());
    }
}

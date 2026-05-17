// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/kerberos/.../ + RFC 4178 (SPNEGO) + RFC 4120 (Kerberos v5)

//! Kerberos GSSAPI + SPNEGO HTTP Negotiate. Port of Keycloak's
//! `federation/kerberos` module and the `SPNEGOAuthenticator`
//! servlet filter.
//!
//! ## What this module covers
//!
//! * [`gssapi`]      — GSSAPI initial + continuation token frame
//!   parsing. Wraps the ASN.1 structure RFC 2743 §3.1 defines —
//!   `0x60` "InitialContextToken" tag containing an OID + the
//!   mechanism-specific blob (SPNEGO).
//! * [`spnego`]      — SPNEGO `NegTokenInit` / `NegTokenResp`
//!   ASN.1 DER parse (RFC 4178 §4).
//! * [`negotiate`]   — HTTP `Negotiate` Authorization header
//!   handler. Issues the 401 + `WWW-Authenticate: Negotiate`
//!   challenge and consumes the client's response.
//! * [`keytab`]      — krb5 keytab file format v0x0502 parser
//!   (the file `kadmin ktadd` produces).
//!
//! ## Honest limitations
//!
//! * **No live KDC integration.** Cave doesn't call a real
//!   `krb5_kt_get_entry` — that requires `libgssapi-sys` or a
//!   from-scratch RFC 4120 client. The crate is structural-only:
//!   parse keytabs, parse SPNEGO frames, build the HTTP 401
//!   challenge. A real production integration adds the
//!   GSS-API call from a sidecar.
//! * **No ticket cryptography.** ENC-TGS-REP, EncTicketPart,
//!   etc. are parsed only to the level RFC 4120 §5 documents
//!   their tag — payload decryption is out of scope.
//! * **No mutual auth follow-up.** Once the client's
//!   `NegTokenResp` arrives we extract the principal and stop;
//!   the optional `mutual_auth` token returned to the client is
//!   built but not signed.

pub mod gssapi;
pub mod gssapi_real;
pub mod keytab;
pub mod negotiate;
pub mod spnego;

#[cfg(test)]
mod tests {
    pub mod libgssapi_real;
    pub mod upstream_port;
}

use thiserror::Error;

/// Surface error type.
#[derive(Debug, Error)]
pub enum KerberosError {
    #[error("Kerberos ASN.1 parse error: {0}")]
    Asn1(String),
    #[error("Kerberos GSSAPI error: {0}")]
    Gssapi(String),
    #[error("Kerberos SPNEGO error: {0}")]
    Spnego(String),
    #[error("Keytab format error: {0}")]
    Keytab(String),
    #[error("Kerberos protocol error: {0}")]
    Protocol(String),
}

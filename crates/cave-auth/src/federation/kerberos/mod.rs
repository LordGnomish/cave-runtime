// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/kerberos
//
// Kerberos / SPNEGO support for cave-auth.
//
// **Honest scope** — implementing Kerberos ticket cryptography in
// pure Rust without a GSSAPI library would require porting:
//
//   * MIT krb5 / Heimdal AES-CTS-HMAC-SHA1-96 (RFC 3962)
//   * AES-CTS-HMAC-SHA2 (RFC 8009) for AD 2008+
//   * Kerberos preauth, FAST, S4U2Proxy, etc.
//
// That's months of work and well-known security minefields.  We
// instead:
//
//   1. Parse keytabs end-to-end (MIT format, "krb5_keytab").  This
//      gives an operator-facing inventory of principals + kvnos +
//      enctypes — the same view `klist -k` shows.
//   2. Parse + serialise the SPNEGO `NegTokenInit` / `NegTokenResp`
//      envelopes (RFC 4178).  This is the HTTP-layer
//      `Authorization: Negotiate ...` exchange.
//   3. Decode the wrapped AP-REQ + AP-REP shapes (RFC 4120) so an
//      operator can see who is calling the cave portal even before
//      we link a real GSSAPI library.
//   4. Surface a `gss_init_sec_context` shim that returns
//      `FederationError::GssapiNotLinked` until a downstream binary
//      decides to link `libgssapi-krb5`.

pub mod keytab;
pub mod principal;
pub mod spnego;

pub use keytab::{Keytab, KeytabEntry};
pub use principal::Principal;
pub use spnego::{AuthState, NegState, NegTokenInit, NegTokenResp, Spnego};

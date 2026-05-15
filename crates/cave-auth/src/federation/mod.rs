// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap + federation/kerberos
//
// `cave_auth::federation` — user federation against external identity stores.
//
// This is a clean-room port of Keycloak's `federation/ldap` Java tree
// plus `federation/kerberos`, translated to native Rust.  The Java
// `LDAPStorageProvider` is split between [`ldap`] and [`provider`] —
// `provider` houses the trait abstraction (so future federation
// backends like NIS, IPA, or LDAPI can plug in), while `ldap` holds the
// concrete BER codec, bind/search/sync pipelines, and AD/OpenLDAP
// quirks.  Kerberos lives in [`kerberos`] and implements the
// keytab parser + SPNEGO state machine.
//
// **Honest scope** — actual `gss_init_sec_context()` calls require
// linking system libgssapi.  This module exposes the SPNEGO protocol
// state machine, the keytab format parser, and the AP-REQ ticket
// decoder.  Cryptographic verification of the ticket against keytab
// keys is not performed in this MVP; callers should hand the decoded
// frame to libgssapi via a higher-level shim once linked.  See
// [`kerberos::spnego::AuthState::AwaitingExternalVerify`].

pub mod kerberos;
pub mod ldap;
pub mod metrics;
pub mod provider;

pub use provider::{Federation, FederationConfig, FederationError, FederationKind, FederationStats};

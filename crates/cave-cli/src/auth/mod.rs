// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak@v22.0.0 federation/{ldap,kerberos}/

//! cavectl auth ldap / kerberos sub-command helpers.
//!
//! The actual CLI surface lives in [`crate::main`] (one mega
//! Subcommand enum) — this module owns the HTTP path strings
//! and the response-shaping helpers, parallel to the
//! per-feature splits other cavectl families use.

pub mod kerberos;
pub mod ldap;

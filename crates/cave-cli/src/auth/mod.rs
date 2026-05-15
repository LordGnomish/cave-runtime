// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak@v22.0.0 services/.../{broker/saml, federation/{ldap,kerberos}}/

//! cavectl `auth` library-side surface.
//!
//! Houses the per-protocol parse helpers `cavectl auth {saml,ldap,kerberos,…}`
//! invokes. Kept here (not under `native/`) so library callers can drive the
//! parsers from tests without pulling in the full CLI binary.

pub mod kerberos;
pub mod ldap;
pub mod saml;

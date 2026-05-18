// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak@v22.0.0 services/.../{broker/saml,
// federation/{ldap,kerberos}, webauthn, protocol/{wsfed,oid4vc}, authorization,
// oauth_endpoints, admin/IdentityProviderResource, admin/AuthenticationManagementResource,
// events/email, model/jpa}/ + webauthn4j@v0.24.0 + RFC 9449 + RFC 7516 +
// RFC 8693 + RFC 8628 + RFC 7009 + RFC 9126 + OpenID CIBA + Kantara UMA 2.0.

//! cavectl `auth` library-side surface.
//!
//! Houses the per-protocol parse helpers `cavectl auth {saml,ldap,kerberos,
//! webauthn,oauth,wsfed,oid4vc,uma,token-exchange,dpop,jwe,admin-idp,
//! admin-flows,email,persistence}` invokes. Kept here (not under `native/`)
//! so library callers can drive the parsers from tests without pulling in
//! the full CLI binary.

pub mod admin_flows;
pub mod admin_idp;
pub mod dpop;
pub mod email_listener;
pub mod jwe;
pub mod kerberos;
pub mod ldap;
pub mod oauth_endpoints;
pub mod oid4vc;
pub mod persistence;
pub mod saml;
pub mod token_exchange;
pub mod uma;
pub mod webauthn;
pub mod wsfed;

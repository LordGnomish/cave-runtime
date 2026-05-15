// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap
//
// `cave_auth::federation::ldap` — RFC 4511 (LDAPv3) client.
//
// The submodules mirror Keycloak's `idm/store/ldap/` layout:
//
// * [`ber`]    — BER/DER codec for the LDAP application class.  Hand
//                rolled (no `rasn` dep in workspace).  Subset
//                sufficient for the LDAPMessage shapes Keycloak emits.
// * [`bind`]   — Simple + SASL (PLAIN/EXTERNAL/GSSAPI) bind requests.
// * [`search`] — SearchRequest + SearchResultEntry/Done + RFC 2696
//                paged-results control.
// * [`sync`]   — `UserStorageSyncTask` port: full vs incremental.
// * [`mapper`] — `group-ldap-mapper`, `role-ldap-mapper`,
//                `user-attribute-ldap-mapper`.
// * [`ad`]     — Active Directory specifics.
// * [`openldap`] — OpenLDAP / 389-ds / RHDS specifics.
// * [`filter`] — RFC 4515 filter parser (subset for sync queries).

pub mod ad;
pub mod ber;
pub mod bind;
pub mod filter;
pub mod mapper;
pub mod object;
pub mod openldap;
pub mod search;
pub mod sync;

pub use object::{LdapAttribute, LdapObject};

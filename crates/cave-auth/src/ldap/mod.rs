// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/.../ + RFC 4511 (LDAPv3)
//
//! LDAP storage federation — port of Keycloak's `federation/ldap`
//! (`LDAPStorageProvider` + `LDAPQueryBuilder` + the AD-specific
//! attribute mapper).
//!
//! This module is *intentionally* pure-Rust BER LDAPv3 — no
//! `ldap3` dependency. The driver explicitly trades the rich
//! client crate (which pulls a TLS stack and a tokio-native
//! reactor) for a small in-house protocol layer that we can
//! verify byte-for-byte against RFC 4511 §4 in unit tests
//! without ever touching the network. A real production LDAP
//! deployment will still wire in TLS through `rustls`; the
//! framework leaves the socket pluggable.
//!
//! ## What this module covers
//!
//! * [`connection`]     — Bind / Unbind / simple-bind state
//!   machine. Tracks an LDAP `messageID` counter the same way
//!   Keycloak's `LDAPIdentityStore` does.
//! * [`query`]          — Search filter builder (`& | ! = ~=
//!   substr present`) and scope enum, modelled on Keycloak's
//!   `LDAPQueryBuilder`.
//! * [`user_mapper`]    — Maps LDAP attributes (`uid`, `mail`,
//!   `cn`, `memberOf`) to cave-auth user fields. Same
//!   `UserAttributeLDAPStorageMapper` design as Keycloak.
//! * [`group_mapper`]   — Two group-sync strategies:
//!   `memberOf` (LDAP entry has a back-reference) and
//!   `member`   (group entry lists its members).
//! * [`storage_provider`] — `UserStorageProvider` trait + the
//!   `LdapStorageProvider` struct that wires connection +
//!   query + mapper into a single federation surface.
//! * [`active_directory`] — AD-specific attribute mapping —
//!   `sAMAccountName`, `userAccountControl` flags,
//!   `objectSid` parse, `pwdLastSet` FILETIME.
//!
//! ## Honest limitations
//!
//! * SASL EXTERNAL is *structurally* representable in
//!   [`connection::BindAuth`] but cannot be exercised without
//!   a real LDAP server providing client-cert auth — the
//!   RFC 4513 §5.2.1 bind path is built; live verification is
//!   tracked as a manifest gap.
//! * StartTLS upgrade (RFC 4513 §3) is *out of scope*. Cave's
//!   deployments wire TLS up-front via LDAPS on 636.
//! * Referrals (RFC 4511 §4.1.10) are surfaced as their own
//!   result code but no recursive follow is implemented —
//!   intentional, matching Keycloak's default policy.

pub mod active_directory;
pub mod connection;
pub mod group_mapper;
pub mod query;
pub mod storage_provider;
pub mod user_mapper;

#[cfg(test)]
mod tests {
    pub mod upstream_port;
}

use thiserror::Error;

/// Errors surfaced by the LDAP federation layer. Modelled on
/// Keycloak's `LDAPStorageProviderException` — the cases callers
/// have to branch on.
#[derive(Debug, Error)]
pub enum LdapError {
    /// BER encode / decode error — a frame was malformed.
    #[error("LDAP BER protocol error: {0}")]
    Protocol(String),
    /// `BindResponse.resultCode != success (0)`.
    #[error("LDAP bind failed: {0}")]
    BindFailed(String),
    /// Search returned a non-success result code.
    #[error("LDAP search failed: {0}")]
    SearchFailed(String),
    /// Required attribute missing from a search result entry.
    #[error("LDAP entry missing attribute: {0}")]
    MissingAttribute(String),
    /// Filter string could not be parsed (RFC 4515 §3).
    #[error("LDAP filter parse error: {0}")]
    FilterParse(String),
    /// Mapper rejected an entry because no required source
    /// attribute was present.
    #[error("LDAP mapper error: {0}")]
    Mapper(String),
}

/// LDAP `resultCode` values from RFC 4511 §4.1.9 — the subset
/// cave-auth has to branch on. Bigger set tracked in
/// [`ResultCode::from_raw`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultCode {
    /// 0 — operation succeeded.
    Success,
    /// 1 — operations error.
    OperationsError,
    /// 49 — invalid credentials.
    InvalidCredentials,
    /// 32 — no such object (DN not found).
    NoSuchObject,
    /// 53 — server unwilling to perform (commonly: weak
    /// password / unencrypted bind blocked).
    UnwillingToPerform,
    /// Other result code — unknown / unhandled.
    Other(u32),
}

impl ResultCode {
    /// Map the wire `INTEGER` from `BindResponse.resultCode`.
    pub fn from_raw(code: u32) -> Self {
        match code {
            0 => ResultCode::Success,
            1 => ResultCode::OperationsError,
            32 => ResultCode::NoSuchObject,
            49 => ResultCode::InvalidCredentials,
            53 => ResultCode::UnwillingToPerform,
            n => ResultCode::Other(n),
        }
    }
    /// Inverse of [`ResultCode::from_raw`].
    pub fn as_raw(self) -> u32 {
        match self {
            ResultCode::Success => 0,
            ResultCode::OperationsError => 1,
            ResultCode::NoSuchObject => 32,
            ResultCode::InvalidCredentials => 49,
            ResultCode::UnwillingToPerform => 53,
            ResultCode::Other(n) => n,
        }
    }
    /// Whether the operation succeeded.
    pub fn is_success(self) -> bool {
        matches!(self, ResultCode::Success)
    }
}

#[cfg(test)]
mod mod_tests {
    use super::*;
    #[test]
    fn result_code_roundtrips_through_raw() {
        for c in [
            ResultCode::Success,
            ResultCode::OperationsError,
            ResultCode::InvalidCredentials,
            ResultCode::NoSuchObject,
            ResultCode::UnwillingToPerform,
            ResultCode::Other(80),
        ] {
            assert_eq!(ResultCode::from_raw(c.as_raw()), c);
        }
    }
    #[test]
    fn success_is_success() {
        assert!(ResultCode::Success.is_success());
        assert!(!ResultCode::InvalidCredentials.is_success());
    }
}

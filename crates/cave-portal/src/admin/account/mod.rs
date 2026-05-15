// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/account-ui/ (visual port, server-rendered Maud)
//
//! `/account/*` — Keycloak Account Console parity.
//!
//! Server-rendered Maud port of the React Account UI (`js/apps/account-ui/`).
//! Six pages plus shared chrome:
//!
//! * [`profile`]      — Personal info (firstName / lastName / email / username / attrs)
//! * [`password`]     — Change password
//! * [`two_factor`]   — TOTP + WebAuthn + recovery codes
//! * [`applications`] — Client applications + consent grants
//! * [`sessions`]     — Active sessions + revoke
//! * [`account_chrome`] — Shared nav strip
//!
//! Persona-wise the account console is **user-scoped, not admin-scoped**:
//! every signed-in caller can see their own data. Cave models this as
//! "any persona above `Anonymous`" — the dev `Anonymous` ctx is rejected.

pub mod account_chrome;
pub mod applications;
pub mod password;
pub mod profile;
pub mod sessions;
pub mod two_factor;

use crate::admin::permission::{Persona, RequestCtx};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AccountError {
    #[error("account console requires an authenticated user")]
    Unauthenticated,
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// Account-console gate. Any authenticated persona (TenantAdmin or
/// PlatformAdmin) sees their own account pages. Anonymous is bounced
/// to the sign-in page upstream — here we surface a typed error.
pub fn require_account_user(ctx: &RequestCtx) -> Result<(), AccountError> {
    if ctx.persona == Persona::Anonymous {
        return Err(AccountError::Unauthenticated);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Permission, Persona, RequestCtx};

    #[test]
    fn gate_rejects_anonymous() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::Anonymous);
        let err = require_account_user(&ctx).unwrap_err();
        assert_eq!(err, AccountError::Unauthenticated);
    }

    #[test]
    fn gate_admits_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[Permission::AuthSessionsRead], Persona::TenantAdmin);
        assert!(require_account_user(&ctx).is_ok());
    }

    #[test]
    fn gate_admits_platform_admin() {
        let ctx = RequestCtx::developer("acme", &[Permission::AuthSessionsRead]);
        // RequestCtx::developer defaults to PlatformAdmin.
        assert_eq!(ctx.persona, Persona::PlatformAdmin);
        assert!(require_account_user(&ctx).is_ok());
    }
}

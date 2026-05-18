// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-acme — RFC 8555 ACMEv2 server reimplementation.
//!
//! Multi-tenant: every Account, Order and Authorization carries the
//! `tenant_id` that owns it. Cross-tenant lookup returns
//! `AcmeError::CrossTenantDenied` rather than silently leaking.
//!
//! Upstream parity: `cert-manager/cert-manager` ACME issuer client + the
//! openbao PKI engine ACME endpoints (`acme/{directory,new-nonce,
//! new-account,new-order,revoke-cert}`).

pub mod account;
pub mod challenge;
pub mod error;
pub mod order;
pub mod server;

pub use account::{Account, AccountStatus, ExternalAccountBinding, Jwk};
pub use challenge::{Challenge, ChallengeStatus, ChallengeType};
pub use error::{AcmeError, AcmeResult};
pub use order::{Authorization, AuthzStatus, Identifier, IdentifierType, Order, OrderStatus};
pub use server::AcmeServer;

pub const MODULE_NAME: &str = "acme";

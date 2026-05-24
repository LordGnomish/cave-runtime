// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-identity — SPIRE v1.15.0 deep-port (scaffold).
//!
//! Workload identity: server + agent + SVID rotation + federation + OIDC discovery.
//!
//! Upstream: spiffe/spire v1.15.0 (Apache-2.0).

pub mod error;
pub mod models;

pub use error::IdentityError;
pub use models::*;

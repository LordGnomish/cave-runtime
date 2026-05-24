// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-identity — SPIRE v1.15.0 deep-port.
//!
//! Workload identity: server + agent + X.509-SVID + JWT-SVID + federation +
//! OIDC discovery + k8s workload attestor.
//!
//! Upstream: spiffe/spire v1.15.0 (Apache-2.0)
//! source_sha: b7db9650aa98598ee7af21d7a75fbab8f6b70d42

pub mod agent_manager;
pub mod attestor;
pub mod bundle;
pub mod error;
pub mod events;
pub mod federation;
pub mod jwt_svid;
pub mod k8s_attestor;
pub mod models;
pub mod oidc;
pub mod policy;
pub mod registration;
pub mod routes;
pub mod server_ca;
pub mod spiffe_id;
pub mod store;
pub mod x509_svid;

pub use error::{IdentityError, Result};
pub use models::*;

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/TokenExchangeGrantType.java + RFC 8693
//
//! OAuth 2.0 Token Exchange (RFC 8693).
//!
//! Grant type: `urn:ietf:params:oauth:grant-type:token-exchange`.
//!
//! Modules:
//!
//! | Module | RFC 8693 section |
//! |--------|------------------|
//! | [`grant`] | §2 — request & response shape |
//! | [`subject_token`] | §2.1 — `subject_token` + `subject_token_type` |
//! | [`actor_token`]   | §2.1 — `actor_token` + `actor_token_type` (delegation) |
//! | [`audience_switch`] | §2.1 — `requested_token_type` + `audience` |
//! | [`policy`] | §4 — which clients may exchange to which audience |

pub mod actor_token;
pub mod audience_switch;
pub mod grant;
pub mod policy;
pub mod subject_token;

pub use grant::{ExchangeError, ExchangeRequest, ExchangeResponse, TokenExchanger};
pub use policy::{ExchangePolicy, PolicyDecision};
pub use subject_token::{SubjectToken, SubjectTokenType};

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Top-level runtime-client surface for Portal → live backend wiring.
//!
//! This sibling of `admin::runtime_client` adds typed clients for the
//! non-apiserver backends Portal admin pages need to materialise. The
//! pattern is identical: a trait + a `reqwest`-driven HTTP impl + an
//! in-memory mock for tests.
//!
//! Modules:
//!
//! * [`auth`] — cave-auth admin REST (Keycloak parity) + SSE events.

pub mod auth;

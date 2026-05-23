// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE DNS — CoreDNS-compatible authoritative and recursive DNS server.
#![allow(dead_code, unused_imports, unused_variables, unused_mut)]
//!
//! Upstream tracking: CoreDNS
//! Features: Full DNS protocol, all record types, DNSSEC, DoT, DoH,
//!           zone transfers (AXFR/IXFR), dynamic updates (RFC 2136),
//!           23 built-in plugins matching the CoreDNS plugin ecosystem.

pub mod api;
pub mod cli;
pub mod config;
pub mod dnssec;
pub mod error;
pub mod observability;
pub mod plugins;
pub mod protocol;
pub mod routes;
pub mod server;
pub mod zone;

pub use config::DnsConfig;
pub use error::{DnsError, DnsResult};
pub use server::DnsServer;

use axum::Router;
use std::sync::Arc;

pub const MODULE_NAME: &str = "dns";

/// Build the axum management Router for embedding in the cave-runtime binary.
pub fn router(zones: Arc<zone::ZoneManager>) -> Router {
    routes::create_router(zones)
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ingestion protocol handlers.
//!
//! Each submodule converts a wire format into `Vec<crate::types::Span>` for
//! storage. The actual HTTP handlers live in `crate::routes`.

pub mod jaeger;
pub mod opencensus;
pub mod otlp;
pub mod zipkin;

use crate::types::Span;

/// Normalise a service name: trim, fall back to "unknown" if empty.
pub(crate) fn normalise_service(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() { "unknown".into() } else { s.into() }
}

/// Epoch microseconds → nanoseconds.
pub(crate) fn us_to_ns(us: i64) -> u64 {
    (us.max(0) as u64).saturating_mul(1_000)
}

/// Epoch milliseconds → nanoseconds.
pub(crate) fn ms_to_ns(ms: i64) -> u64 {
    (ms.max(0) as u64).saturating_mul(1_000_000)
}

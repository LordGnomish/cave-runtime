// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cross-cutting **security hardening** for cave-portal.
//!
//! Gap-2 of the 2026-05-18 user-friendly-and-secure sprint.
//!
//! Four pillars:
//!
//!   * [`headers`] — OWASP Secure-Headers v2024.04: CSP / HSTS / XFO /
//!     XCTO / Referrer-Policy / Permissions-Policy / COOP / CORP.
//!   * [`csrf`] — random 192-bit base64url tokens + constant-time
//!     double-submit-cookie validator.
//!   * [`cookie`] — `Secure; HttpOnly; SameSite=...; Path=/` attr
//!     builder for every session-bearing cookie we emit.
//!   * [`ratelimit`] — in-memory per-key token bucket with monotonic
//!     refill window + `Retry-After` reporting.
//!
//! These are infrastructure helpers; the calling layer (cave-runtime
//! HTTP wiring) decides which routes get wrapped. They live as a
//! standalone `pub mod` (not under `admin::`) because they are
//! request-time middleware, not server-rendered admin pages.

pub mod cookie;
pub mod csrf;
pub mod headers;
pub mod ratelimit;

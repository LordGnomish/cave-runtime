//! CAVE kernel — shared infrastructure used by all CAVE modules.
//!
//! Modules:
//! - `parity` — upstream parity tracking, manifest parsing, and metric calculation
//! - `ratelimiter` — token + leaky bucket rate limiting (per-tenant)
//! - `circuitbreaker` — Closed/Open/HalfOpen state machine (per-key)
//! - `retrypolicy` — exponential backoff with jitter, async retry executor

pub mod circuitbreaker;
pub mod parity;
pub mod ratelimiter;
pub mod retrypolicy;

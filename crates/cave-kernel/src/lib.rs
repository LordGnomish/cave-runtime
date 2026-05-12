//! CAVE kernel — shared infrastructure used by all CAVE modules.
//!
//! Modules:
//! - `parity` — upstream parity tracking, manifest parsing, and metric calculation
//! - `ratelimiter` — token + leaky bucket rate limiting (per-tenant)
//! - `circuitbreaker` — Closed/Open/HalfOpen state machine (per-key)
//! - `retrypolicy` — exponential backoff with jitter, async retry executor
//! - `consensus` — Raft log/state-machine/handle traits (impl in cave-ha)
//! - `eventbus` — type-safe in-process pub/sub for SSE/watch fan-out
//! - `reconcile` — generic Kubernetes-style reconcile loop runner
//! - `identity` — SPIFFE ID + SVID metadata
//! - `ns` — TenantId / TenantScope newtypes
//! - `codec` — `FrameCodec` trait + length-prefix framing helper used by
//!   `cave-rdbms`, `cave-docdb`, and `cave-cache` wire servers

pub mod backoff;
pub mod circuitbreaker;
pub mod codec;
pub mod consensus;
pub mod eventbus;
pub mod identity;
pub mod lease;
pub mod ns;
pub mod observability;
pub mod parity;
pub mod ratelimiter;
pub mod reconcile;
pub mod retrypolicy;
pub mod semaphore;

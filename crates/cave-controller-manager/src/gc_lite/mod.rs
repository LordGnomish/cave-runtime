//! Light-weight GC controllers — separate from the central GarbageCollector
//! because they operate on narrower object kinds with simpler triggers.
//!
//! * [`podgc`] — terminated pod cleanup (`pkg/controller/podgc`).
//! * [`ttl_after_finished`] — finished-Job TTL cleanup
//!   (`pkg/controller/ttlafterfinished`).

pub mod podgc;
pub mod podgc_deeper;
pub mod ttl_after_finished;
pub mod ttl_jitter;

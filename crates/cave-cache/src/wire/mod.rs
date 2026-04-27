//! Sans-IO RESP2 / RESP3 wire layer.
//!
//! This is the unit-testable counterpart to the existing `crate::resp` module
//! (which couples directly to `tokio::net` half-sockets). The codec here lives
//! in `BytesMut` only, mirroring `redis-rs::parser` and `valkey-glide`'s
//! `protocol` crate. Layered on top:
//!
//! * [`pipeline`]  — buffered request batching + reply correlation.
//! * [`pubsub`]    — channel + pattern fanout state machine.
//! * [`xstream`]   — XADD/XREAD/XREADGROUP/XACK consumer-group semantics.
//! * [`tenant`]    — default-deny tenant ACL (mirrors cave-pg::wire::tenant).
//! * [`pqc`]       — ML-KEM-768 transport KEM negotiation envelope.
//!
//! All modules are `#![cfg_attr(test, allow(...))]` clean and run with
//! `cargo test -p cave-cache --lib wire`.

pub mod codec;
pub mod pipeline;
pub mod pqc;
pub mod pubsub;
pub mod tenant;
pub mod xstream;

pub use codec::{RespValue, RespCodecError, decode, encode};

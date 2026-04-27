//! PostgreSQL frontend/backend protocol v3.0 — wire-level codec.
//!
//! Implements the binary message framing documented at
//! <https://www.postgresql.org/docs/current/protocol.html> and mirrored in
//! the [`pgwire`](https://github.com/sunng87/pgwire) crate. Each module
//! handles one logical phase:
//!
//! * [`startup`]  — startup, SSL/GSS, cancel, parameter negotiation.
//! * [`auth`]     — Authentication{Ok, Cleartext, MD5, SASL/SCRAM-SHA-256(+)}.
//! * [`simple_query`] — `Q` simple-query → RowDescription / DataRow / CommandComplete.
//! * [`extended_query`] — Parse/Bind/Describe/Execute/Sync/Close/Flush.
//! * [`copy`]     — CopyIn/Out/Both, CopyData, CopyDone, CopyFail.
//! * [`replication`] — START_REPLICATION subset (XLogData, KeepAlive, StandbyStatusUpdate).
//! * [`pqc`]      — ML-KEM-768 transport KEM negotiation envelope.
//! * [`tenant`]   — per-tenant isolation (default-deny).
//! * [`pg_types`] — built-in type OIDs and binary encodings used by the codec.
//!
//! All messages are encoded into / decoded from [`bytes::BytesMut`]. The codec
//! is sans-IO: no async, no sockets — callers feed bytes in and pull
//! [`FrontendMessage`] / [`BackendMessage`] values out, mirroring how
//! `tokio-postgres` and `pgwire` separate framing from transport.

pub mod auth;
pub mod codec;
pub mod copy;
pub mod extended_query;
pub mod pg_types;
pub mod pqc;
pub mod replication;
pub mod simple_query;
pub mod startup;
pub mod tenant;

pub use codec::{
    BackendMessage, FrontendMessage, ProtocolError, ProtocolResult, decode_backend,
    decode_frontend, encode_backend, encode_frontend,
};

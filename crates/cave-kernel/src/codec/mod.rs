//! Frame codec primitives shared by wire-protocol crates.
//!
//! Three CAVE crates speak distinct wire protocols (PostgreSQL v3 in
//! `cave-rdbms`, MongoDB OP_MSG in `cave-docdb`, Redis RESP3 in
//! `cave-cache`) but their I/O loops have to solve the same problem:
//! consume bytes off a TCP buffer, attempt to decode one frame, return
//! `None` if more bytes are needed, surface the same error taxonomy on
//! malformed input.
//!
//! This module exposes the abstract [`FrameCodec`] trait plus the parts
//! of the implementation that genuinely repeat across crates:
//!
//! - [`FrameError`] — common error taxonomy (incomplete, invalid, limit,
//!   io). Each crate maps protocol-specific errors into `Invalid`.
//! - [`length_prefix`] — helper for binary length-prefix framing. Used by
//!   PostgreSQL and MongoDB. **Not** used by RESP3, which is line-oriented.
//!
//! The trait itself is intentionally narrow. Concrete state machines
//! (PG message dispatch, OP_MSG section parsing, RESP3 type-prefix
//! recursion) remain in their owning crate — they don't share enough
//! shape to extract usefully.

pub mod frame;
pub mod length_prefix;

pub use frame::{FrameCodec, FrameError};
pub use length_prefix::{Endian, LengthIncludes, LengthSpec, try_read_length_prefixed};

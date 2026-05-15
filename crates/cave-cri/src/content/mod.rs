//! Content-addressable storage (CAS).
//!
//! Ports `core/content/` from upstream containerd
//! (`containerd/containerd@v1.7`). The shape is:
//!
//! * [`store::ContentStore`] — trait every backend implements.
//! * [`store::LocalStore`] — filesystem-backed implementation, the
//!   one cave-cri ships today. Blobs live at
//!   `<root>/blobs/sha256/<hex>` and metadata lives in an in-memory
//!   map keyed by digest. The path layout matches containerd's so an
//!   external tool can still introspect.
//! * [`digest::Digest`] — `sha256` / `sha512` / `sha384` algorithm
//!   wrappers, validated on construction.
//! * [`writer::Writer`] — streaming ingest path: caller writes bytes,
//!   the writer hashes on the fly, then `commit` validates the
//!   expected digest and moves the blob into the store. Aborted
//!   writers leave no trace.
//!
//! Garbage collection: a content blob is GC-eligible iff no
//! [`leases::Lease`] references it (see sibling `leases/` module).
//! That coupling is intentional — the upstream container ecosystem
//! lives on the assumption that content can't be reaped while a
//! lease holds it open.
//!
//! Scope cuts (deliberate, documented in the manifest):
//! * Cross-process locking — single-process per host, the existing
//!   cave-cri invariant.
//! * Boltdb metadata persistence — in-memory + on-disk blobs
//!   suffices for the MVP; metadata reconstructs from a directory
//!   walk on restart.

pub mod digest;
pub mod store;
pub mod writer;

pub use digest::{Digest, DigestAlgorithm, DigestError};
pub use store::{
    ContentInfo, ContentStore, LocalStore, ReaderAtSize, StoreError,
};
pub use writer::{Writer, WriterError};

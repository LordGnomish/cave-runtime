// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Layer diff service.
//!
//! Ports `core/diff/` from upstream containerd. Two operations:
//!
//! * **Apply** — read a gzipped tarball and unpack it into a target
//!   directory, observing the OCI whiteout convention (entries named
//!   `.wh.<name>` delete the corresponding file from the lower
//!   layer). Implemented in [`walking_differ`].
//! * **Compress** — gzip-encode an arbitrary byte stream, returning
//!   the encoded bytes plus the SHA-256 digest of the *uncompressed*
//!   form (containerd terminology: the "diff id"). Implemented in
//!   [`compression`].
//! * **Diff (production)** — the double-tree differ that compares a
//!   `lower` snapshot against an `upper` rootfs and serialises the
//!   change set into an OCI layer tarball (additions/modifications
//!   carry the upper bytes; deletions become `.wh.<name>` whiteouts).
//!   This is the `Compare` half of `core/diff/walking/differ.go`,
//!   layered on `continuity/fs.Changes`. Implemented in [`producer`].
//!   The round-trip invariant — `apply(write_diff_tar(lower, upper))`
//!   over a copy of `lower` reproduces `upper` — ties it to the Apply
//!   side above.

pub mod compression;
pub mod producer;
pub mod walking_differ;

pub use compression::{compress_gzip, compute_diff_id, decompress_gzip, CompressionError};
pub use producer::{compute_changes, diff_layer, write_diff_tar, Change, ChangeKind, DiffError, DiffLayer};
pub use walking_differ::{apply_layer, ApplyError, ApplyStats};

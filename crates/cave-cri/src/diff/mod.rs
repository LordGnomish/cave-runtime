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
//!
//! Scope cut: the production path (diff-of-two-filesystems, the
//! `Differ.Diff` operation that produces an upper-layer tarball from
//! two overlayfs branches) requires a deep walk of two directory
//! trees. The walking_differ implementation here covers the read
//! side (Apply) that container start hits, plus a synthesizing
//! single-directory "diff" that callers can hash. The double-tree
//! diff is left for a follow-up.

pub mod compression;
pub mod walking_differ;

pub use compression::{compress_gzip, compute_diff_id, decompress_gzip, CompressionError};
pub use walking_differ::{apply_layer, ApplyError, ApplyStats};

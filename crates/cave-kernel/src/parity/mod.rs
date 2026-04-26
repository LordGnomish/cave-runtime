//! Upstream parity tracking for CAVE modules.
//!
//! Each CAVE module is a line-for-line reimplementation of an upstream OSS tool.
//! This module provides honest, filesystem-backed parity metrics:
//!
//! - **file_parity**     — which upstream source files have a local counterpart
//! - **function_parity** — which upstream functions/methods are implemented
//! - **test_parity**     — which upstream tests have been ported
//! - **surface_parity**  — which HTTP/gRPC/CLI endpoints are wired up
//! - **stubs_detected**  — raw count of `todo!` / `unimplemented!` in the source tree
//!
//! Each module declares its intent in a `parity.manifest.toml` at the crate root.
//! The `ParityCalculator` reads the manifest and resolves each mapping against the
//! actual file-system to produce a `ParityReport`.

pub mod calculator;
pub mod manifest;
pub mod types;

pub use calculator::{FsResolver, ParityCalculator};
pub use manifest::ParityManifest;
pub use types::{GapItem, GapKind, ParityMetric, ParityReport};

/// Parse a manifest TOML string and run the calculator against the filesystem rooted at
/// `base_path` (typically `env!("CARGO_MANIFEST_DIR")` of the calling crate).
pub fn calculate_from_str(
    manifest_toml: &str,
    base_path: &str,
) -> Result<ParityReport, toml::de::Error> {
    let manifest: ParityManifest = toml::from_str(manifest_toml)?;
    let resolver = FsResolver::new(base_path);
    Ok(ParityCalculator::new(resolver).calculate(&manifest))
}
